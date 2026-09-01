//! Redaction that preserves the document (no rasterising): delete text glyphs
//! and replace embedded images.
//!
//! [`redact_text`](crate::Pdf::redact_text) **deletes** the glyphs of detected
//! spans from the content streams (rather than re-encoding a replacement, which
//! corrupts subset/CID fonts) and strips the structures that retain copies of
//! the text (annotations, `/Info`, `/Metadata`), keeping a real selectable text
//! layer with the detected spans gone. `redact_images` (feature `image`)
//! replaces an embedded image XObject with a redacted image.
//!
//! Pure-Rust: no renderer, no font subsetting. The glyph decode reuses lopdf's
//! per-font [`Encoding`], walked one glyph at a time so a detected character
//! span maps to exact glyph byte ranges.

mod glyphs;
#[cfg(feature = "image")]
mod images;
mod sanitize;

use std::collections::BTreeMap;

use lopdf::content::Content;
use lopdf::{Encoding, Object, ObjectId};

use self::glyphs::decode_glyphs;
#[cfg(feature = "image")]
pub use self::images::ImageReplacement;
use crate::Pdf;
use crate::error::{Error, Result};

/// A detected span to redact: a character range into a page's text, as produced
/// by [`page_texts`](Pdf::page_texts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    /// 1-based page number.
    pub page: u32,
    /// Start character offset into the page text (inclusive).
    pub start: usize,
    /// End character offset into the page text (exclusive).
    pub end: usize,
}

impl Detection {
    /// A detection of `[start, end)` on `page`.
    pub fn new(page: u32, start: usize, end: usize) -> Self {
        Self { page, start, end }
    }
}

/// One page's text with the per-character source needed to map a detected span
/// back to the glyphs that drew it.
struct PageText {
    page: u32,
    /// The page's indirect-object id (`lopdf`'s `(object number, generation)`),
    /// distinct from the 1-based `page` number: it addresses the page object for
    /// content get/set.
    page_id: ObjectId,
    /// The page text, char for char aligned with `glyphs`.
    text: String,
    /// One entry per character of `text`, naming the glyph that produced it.
    /// Characters with no glyph (synthetic spaces between text runs) are `None`.
    per_char: Vec<Option<GlyphRef>>,
}

/// Address of the string that draws text: which content operation, which
/// operand, and which string *within* that operand (for a `TJ` array, or `None`
/// for a plain `Tj` string).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct GlyphSite {
    op: usize,
    operand: usize,
    /// Index of the string within a `TJ` array operand, or `None` for a plain
    /// `Tj` string operand.
    item: Option<usize>,
}

/// Glyph byte ranges to delete, grouped by the string they live in.
type Deletions = BTreeMap<GlyphSite, Vec<(usize, usize)>>;

/// Where a character's glyph lives: the string that drew it and the glyph's byte
/// range within that string.
#[derive(Debug, Clone, Copy)]
struct GlyphRef {
    site: GlyphSite,
    byte_start: usize,
    byte_end: usize,
}

/// A page's text under construction, char for char aligned with the per-char
/// glyph map: [`push_char`](TextRun::push_char) appends a decoded character with
/// its originating glyph, [`push_gap`](TextRun::push_gap) a synthetic space with
/// no glyph.
struct TextRun<'a> {
    text: &'a mut String,
    per_char: &'a mut Vec<Option<GlyphRef>>,
}

impl TextRun<'_> {
    /// Append a decoded character tagged with the glyph that drew it.
    fn push_char(&mut self, ch: char, glyph: GlyphRef) {
        self.text.push(ch);
        self.per_char.push(Some(glyph));
    }

    /// Append a synthetic space (a word gap or `TJ`-array trailing space) that
    /// no glyph drew.
    fn push_gap(&mut self) {
        self.text.push(' ');
        self.per_char.push(None);
    }

    /// Append the text a `Tj`/`TJ` operand list draws, recording each character's
    /// originating glyph. Mirrors lopdf's `collect_text`: a `TJ` array's strings
    /// are decoded in order, and a large-negative kerning number inserts a space
    /// (with no glyph).
    fn show_text(&mut self, enc: &Encoding, operands: &[Object], op: usize) -> Result<()> {
        for (operand, value) in operands.iter().enumerate() {
            match value {
                Object::String(bytes, _) => {
                    let site = GlyphSite {
                        op,
                        operand,
                        item: None,
                    };
                    self.push_glyphs(enc, bytes, site)?;
                }
                Object::Array(arr) => {
                    // A `TJ` array interleaves strings with kerning adjustments (in
                    // thousandths of an em, negated). A large-negative adjustment is
                    // a word gap and reads as a space. The exact threshold is not
                    // font-metric-precise, but it deliberately mirrors lopdf's own
                    // text extraction so the page text `page_texts` returns — the
                    // string a caller runs detection over — matches character for
                    // character; the `per_char` glyph map is built in the same pass
                    // with the same rule, so a detected span stays aligned with the
                    // glyphs that drew it. The array is then followed by a trailing
                    // space, also matching lopdf.
                    const WORD_GAP_KERN: f64 = -100.0;
                    for (item_idx, item) in arr.iter().enumerate() {
                        match item {
                            Object::String(bytes, _) => {
                                let site = GlyphSite {
                                    op,
                                    operand,
                                    item: Some(item_idx),
                                };
                                self.push_glyphs(enc, bytes, site)?;
                            }
                            Object::Integer(i) if (*i as f64) < WORD_GAP_KERN => self.push_gap(),
                            Object::Real(f) if (*f as f64) < WORD_GAP_KERN => self.push_gap(),
                            _ => {}
                        }
                    }
                    self.push_gap();
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Decode `bytes` into glyphs and append their characters, each tagged with
    /// the glyph that drew it.
    fn push_glyphs(&mut self, enc: &Encoding, bytes: &[u8], site: GlyphSite) -> Result<()> {
        for glyph in decode_glyphs(enc, bytes)? {
            for ch in glyph.text.chars() {
                self.push_char(
                    ch,
                    GlyphRef {
                        site,
                        byte_start: glyph.byte_start,
                        byte_end: glyph.byte_end,
                    },
                );
            }
        }
        Ok(())
    }
}

impl Pdf {
    /// The text of every page, for a caller to run detection over. The returned
    /// offsets are the character offsets [`redact_text`](Pdf::redact_text)
    /// expects in its [`Detection`]s.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if a
    ///   page's content or fonts cannot be read;
    /// - [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a page
    ///   draws text with a font whose encoding cannot be decoded — that text
    ///   cannot be mapped to glyphs for redaction, so it is surfaced as an error
    ///   rather than silently omitted.
    pub fn page_texts(&self) -> Result<Vec<(u32, String)>> {
        Ok(self
            .page_texts_inner()?
            .into_iter()
            .map(|p| (p.page, p.text))
            .collect())
    }

    /// Redact `detections` by deleting the glyphs that drew them, then sanitise
    /// the document (strip annotations, form values, embedded files, the
    /// outline, `/Info`, and `/Metadata`), returning the
    /// new bytes.
    ///
    /// The output keeps a selectable text layer: only the detected glyphs are
    /// removed, with the original fonts and remaining text intact. Because it
    /// deletes rather than re-encodes, it does not corrupt subset/CID fonts.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if the
    ///   document cannot be read or re-saved;
    /// - [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a page
    ///   draws text with an undecodable font (see [`page_texts`](Pdf::page_texts)) —
    ///   redaction is refused rather than silently leaving that text in place.
    pub fn redact_text(&self, detections: &[Detection]) -> Result<Vec<u8>> {
        let pages = self.page_texts_inner()?;
        let mut doc = self.doc.clone();

        for page in &pages {
            let dels: Vec<&Detection> = detections.iter().filter(|d| d.page == page.page).collect();
            if dels.is_empty() {
                continue;
            }

            // Collect glyph byte ranges to delete, grouped by the exact string
            // they live in: (op, operand, item-within-TJ-array).
            let mut to_delete: Deletions = BTreeMap::new();
            for d in dels {
                for ch in d.start..d.end {
                    if let Some(Some(g)) = page.per_char.get(ch) {
                        to_delete
                            .entry(g.site)
                            .or_default()
                            .push((g.byte_start, g.byte_end));
                    }
                }
            }
            if to_delete.is_empty() {
                continue;
            }

            let content_data = doc.get_page_content(page.page_id);
            let mut content = Content::decode(&content_data)
                .map_err(|e| Error::invalid_document(format!("decode page content: {e}")))?;
            apply_deletions(&mut content, &to_delete);
            let new_content = content
                .encode()
                .map_err(|e| Error::invalid_document(format!("encode page content: {e}")))?;
            doc.change_page_content(page.page_id, new_content)
                .map_err(|e| Error::invalid_document(format!("write page content: {e}")))?;
        }

        sanitize::sanitize(&mut doc);

        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|e| Error::invalid_document(format!("save redacted PDF: {e}")))?;
        Ok(out)
    }

    /// Build the per-page text and per-character glyph map by walking each
    /// page's content in operator order — the same order lopdf's text
    /// extraction uses — so a character offset maps to the glyph that drew it.
    fn page_texts_inner(&self) -> Result<Vec<PageText>> {
        let pages = self.doc.get_pages();
        let mut out = Vec::with_capacity(pages.len());

        for (&page, &page_id) in &pages {
            let fonts = self
                .doc
                .get_page_fonts(page_id)
                .map_err(|e| Error::invalid_document(format!("page {page} fonts: {e}")))?;
            // Resolve every font's encoding. A font whose encoding cannot be
            // resolved maps to `None` (not omitted): if text is later drawn with
            // it, that text is undecodable and redaction must fail closed rather
            // than silently leave it in the output.
            let encodings: BTreeMap<Vec<u8>, Option<Encoding>> = fonts
                .into_iter()
                .map(|(name, font)| (name, font.get_font_encoding(&self.doc).ok()))
                .collect();

            let content_data = self.doc.get_page_content(page_id);
            let content = Content::decode(&content_data)
                .map_err(|e| Error::invalid_document(format!("page {page} content: {e}")))?;

            let mut text = String::new();
            let mut per_char: Vec<Option<GlyphRef>> = Vec::new();
            let mut run = TextRun {
                text: &mut text,
                per_char: &mut per_char,
            };
            // The current font's resolved encoding: `None` before any `Tf`, or
            // `Some(None)` when the selected font could not be decoded or names
            // a font absent from the page's resources.
            const UNRESOLVED: &Option<Encoding> = &None;
            let mut current: Option<&Option<Encoding>> = None;

            for (op_idx, op) in content.operations.iter().enumerate() {
                match op.operator.as_str() {
                    "Tf" => {
                        if let Some(Object::Name(name)) = op.operands.first() {
                            // A `Tf` naming a font not in the resources is an
                            // unresolved selection, held as `Some(None)` so the
                            // next text op fails closed rather than reading as
                            // "no font selected".
                            current = Some(encodings.get(name).unwrap_or(UNRESOLVED));
                        }
                    }
                    "Tj" | "TJ" => match current {
                        Some(Some(enc)) => {
                            run.show_text(enc, &op.operands, op_idx)?;
                        }
                        // Text drawn under a font we could not decode or resolve:
                        // fail closed — its glyphs cannot be located for
                        // deletion, so redacting this document would silently
                        // leave them in.
                        Some(None) => {
                            return Err(Error::unsafe_rewrite(format!(
                                "page {page} draws text with a font whose encoding \
                                 could not be decoded or was not found in the page \
                                 resources; its text cannot be redacted"
                            )));
                        }
                        // No font selected yet (malformed stream): skip.
                        None => {}
                    },
                    _ => {}
                }
            }

            out.push(PageText {
                page,
                page_id,
                text,
                per_char,
            });
        }
        Ok(out)
    }
}

/// Remove the marked glyph byte ranges from each exact string, high-to-low so
/// earlier offsets stay valid.
fn apply_deletions(content: &mut Content, to_delete: &Deletions) {
    for (&GlyphSite { op, operand, item }, ranges) in to_delete {
        let Some(op) = content.operations.get_mut(op) else {
            continue;
        };
        let target: Option<&mut Vec<u8>> = match (op.operands.get_mut(operand), item) {
            (Some(Object::String(bytes, _)), None) => Some(bytes),
            (Some(Object::Array(arr)), Some(k)) => match arr.get_mut(k) {
                Some(Object::String(bytes, _)) => Some(bytes),
                _ => None,
            },
            _ => None,
        };
        if let Some(bytes) = target {
            // The same glyph range can be collected more than once (a ligature
            // whose one code spans several detected characters, or overlapping
            // detections). Merge overlapping/adjacent ranges so each byte span
            // is drained exactly once — draining a span twice would corrupt the
            // string by consuming later, still-valid bytes.
            let mut merged: Vec<(usize, usize)> = ranges.clone();
            merged.sort_unstable();
            merged.dedup();
            let mut coalesced: Vec<(usize, usize)> = Vec::with_capacity(merged.len());
            for (s, e) in merged {
                match coalesced.last_mut() {
                    Some(last) if s <= last.1 => last.1 = last.1.max(e),
                    _ => coalesced.push((s, e)),
                }
            }
            // Drain high-to-low so earlier offsets stay valid.
            for &(s, e) in coalesced.iter().rev() {
                if e <= bytes.len() && s < e {
                    bytes.drain(s..e);
                }
            }
        }
    }
}
