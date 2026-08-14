//! Redaction that preserves the document (no rasterising): delete text glyphs
//! and replace embedded images.
//!
//! [`redact_text`](crate::Pdf::redact_text) **deletes** the glyphs of detected
//! spans from the content streams (rather than re-encoding a replacement, which
//! corrupts subset/CID fonts) and strips the structures that retain copies of
//! the text (annotations, `/Info`, `/Metadata`), keeping a real selectable text
//! layer with the detected spans gone. [`redact_images`](crate::Pdf::redact_images)
//! (feature `image`) replaces an embedded image XObject with a redacted image.
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
use lopdf::{Encoding, Object};

use self::glyphs::{Glyph, decode_glyphs};
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
    page_id: (u32, u16),
    /// The page text, char for char aligned with `glyphs`.
    text: String,
    /// One entry per character of `text`, naming the glyph that produced it.
    /// Characters with no glyph (synthetic spaces between text runs) are `None`.
    per_char: Vec<Option<GlyphRef>>,
}

/// Address of a string that draws text: `(operation index, operand index,
/// item index within a `TJ` array or `None` for a `Tj` string)`.
type StringAddr = (usize, usize, Option<usize>);

/// Glyph byte ranges to delete, grouped by the string they live in.
type Deletions = BTreeMap<StringAddr, Vec<(usize, usize)>>;

/// Where a character's glyph lives: which content operation, which operand,
/// which string *within* that operand (for a `TJ` array), and the glyph's byte
/// range within that string.
#[derive(Debug, Clone, Copy)]
struct GlyphRef {
    op: usize,
    operand: usize,
    /// Index of the string within a `TJ` array operand, or `None` for a plain
    /// `Tj` string operand.
    item: Option<usize>,
    byte_start: usize,
    byte_end: usize,
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
                            .entry((g.op, g.operand, g.item))
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
            // The current font's resolved encoding: `None` before any `Tf`, or
            // `Some(None)` when the selected font could not be decoded.
            let mut current: Option<&Option<Encoding>> = None;

            for (op_idx, op) in content.operations.iter().enumerate() {
                match op.operator.as_str() {
                    "Tf" => {
                        if let Some(Object::Name(name)) = op.operands.first() {
                            current = encodings.get(name);
                        }
                    }
                    "Tj" | "TJ" => match current {
                        Some(Some(enc)) => {
                            show_text(enc, &op.operands, op_idx, &mut text, &mut per_char);
                        }
                        // Text drawn under a font we could not decode: fail
                        // closed — its glyphs cannot be located for deletion, so
                        // redacting this document would silently leave them in.
                        Some(None) => {
                            return Err(Error::unsafe_rewrite(format!(
                                "page {page} draws text with a font whose encoding \
                                 could not be decoded; its text cannot be redacted"
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

/// Append the text a `Tj`/`TJ` operand list draws, recording each character's
/// originating glyph. Mirrors lopdf's `collect_text`: a `TJ` array's strings are
/// decoded in order, and a large-negative kerning number inserts a space (with
/// no glyph).
fn show_text(
    enc: &Encoding,
    operands: &[Object],
    op_idx: usize,
    text: &mut String,
    per_char: &mut Vec<Option<GlyphRef>>,
) {
    for (operand_idx, operand) in operands.iter().enumerate() {
        match operand {
            Object::String(bytes, _) => {
                push_glyphs(enc, bytes, op_idx, operand_idx, None, text, per_char);
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
                        Object::String(bytes, _) => push_glyphs(
                            enc,
                            bytes,
                            op_idx,
                            operand_idx,
                            Some(item_idx),
                            text,
                            per_char,
                        ),
                        Object::Integer(i) if (*i as f64) < WORD_GAP_KERN => {
                            text.push(' ');
                            per_char.push(None);
                        }
                        Object::Real(f) if (*f as f64) < WORD_GAP_KERN => {
                            text.push(' ');
                            per_char.push(None);
                        }
                        _ => {}
                    }
                }
                text.push(' ');
                per_char.push(None);
            }
            _ => {}
        }
    }
}

/// Decode `bytes` into glyphs and append their characters, each tagged with the
/// glyph that drew it.
fn push_glyphs(
    enc: &Encoding,
    bytes: &[u8],
    op: usize,
    operand: usize,
    item: Option<usize>,
    text: &mut String,
    per_char: &mut Vec<Option<GlyphRef>>,
) {
    for glyph in decode_glyphs(enc, bytes) {
        let Glyph {
            byte_start,
            byte_end,
            text: gt,
        } = glyph;
        for ch in gt.chars() {
            text.push(ch);
            per_char.push(Some(GlyphRef {
                op,
                operand,
                item,
                byte_start,
                byte_end,
            }));
        }
    }
}

/// Remove the marked glyph byte ranges from each exact string, high-to-low so
/// earlier offsets stay valid.
fn apply_deletions(content: &mut Content, to_delete: &Deletions) {
    for (&(op_idx, operand_idx, item), ranges) in to_delete {
        let Some(op) = content.operations.get_mut(op_idx) else {
            continue;
        };
        let target: Option<&mut Vec<u8>> = match (op.operands.get_mut(operand_idx), item) {
            (Some(Object::String(bytes, _)), None) => Some(bytes),
            (Some(Object::Array(arr)), Some(k)) => match arr.get_mut(k) {
                Some(Object::String(bytes, _)) => Some(bytes),
                _ => None,
            },
            _ => None,
        };
        if let Some(bytes) = target {
            let mut sorted: Vec<(usize, usize)> = ranges.clone();
            sorted.sort_unstable_by_key(|&(start, _)| std::cmp::Reverse(start));
            for (s, e) in sorted {
                if e <= bytes.len() && s < e {
                    bytes.drain(s..e);
                }
            }
        }
    }
}
