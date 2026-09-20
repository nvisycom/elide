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
mod tounicode;

use std::collections::{BTreeMap, BTreeSet};

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
    /// The resource names of the fonts referenced by `per_char`, indexed by
    /// [`GlyphRef::font`]. Used to reach a font's `/ToUnicode` CMap.
    fonts: Vec<Vec<u8>>,
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
    /// Index into the page's font-name table ([`PageText::fonts`]) naming the
    /// font that drew the glyph, so a deleted glyph's raw code can be scrubbed
    /// from that font's `/ToUnicode` CMap.
    font: u16,
}

/// A page's text under construction, char for char aligned with the per-char
/// glyph map: [`push_char`](TextRun::push_char) appends a decoded character with
/// its originating glyph, [`push_gap`](TextRun::push_gap) a synthetic space with
/// no glyph.
struct TextRun<'a> {
    text: &'a mut String,
    per_char: &'a mut Vec<Option<GlyphRef>>,
    /// The font-table index of the currently selected font, stamped onto every
    /// glyph pushed so a deletion can later reach that font's `/ToUnicode`.
    font: u16,
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
    ///
    /// `operand_base` is the index of `operands[0]` within the operation's full
    /// operand list, so the recorded [`GlyphSite`] addresses the true operand
    /// even when the caller passes a sub-slice (as `"` does, whose string is
    /// operand 2). It is 0 when the whole operand list is passed.
    fn show_text(
        &mut self,
        enc: &Encoding,
        operands: &[Object],
        op: usize,
        operand_base: usize,
    ) -> Result<()> {
        for (i, value) in operands.iter().enumerate() {
            let operand = operand_base + i;
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
                    // text extraction so the page text `page_texts` returns, the
                    // string a caller runs detection over, matches character for
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
                        font: self.font,
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
    ///   draws text with a font whose encoding cannot be decoded, that text
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
    ///   draws text with an undecodable font (see [`page_texts`](Pdf::page_texts)),
    ///   redaction is refused rather than silently leaving that text in place.
    pub fn redact_text(&self, detections: &[Detection]) -> Result<Vec<u8>> {
        let pages = self.page_texts_inner()?;
        let mut doc = self.doc.clone();

        // Raw glyph codes deleted from each font, keyed by that font's
        // `/ToUnicode` stream object id, gathered across pages so a shared CMap is
        // scrubbed once after the content edits. Fonts without a `/ToUnicode`
        // contribute nothing (there is no code->Unicode table to leak).
        let mut deleted_codes: BTreeMap<ObjectId, BTreeSet<Vec<u8>>> = BTreeMap::new();
        // Codes still drawn by surviving text, per CMap. A code is only scrubbed
        // from a `/ToUnicode` if no surviving glyph anywhere uses it: fonts and
        // their CMaps are routinely shared, and the same code (e.g. the letter
        // `a`) recurs in text that stays, so scrubbing a still-used code would
        // corrupt the surviving text's extraction.
        let mut surviving_codes: BTreeMap<ObjectId, BTreeSet<Vec<u8>>> = BTreeMap::new();

        // First pass over every page: record the codes of glyphs that will
        // survive, so the scrub can spare any code still in use.
        for page in &pages {
            let deleted_here: BTreeSet<usize> = detections
                .iter()
                .filter(|d| d.page == page.page)
                .flat_map(|d| d.start..d.end)
                .collect();
            let font_tounicode = tounicode::page_font_cmaps(&doc, page.page_id);
            if font_tounicode.is_empty() {
                continue;
            }
            let content = Content::decode(&doc.get_page_content(page.page_id))
                .map_err(|e| Error::invalid_document(format!("decode page content: {e}")))?;
            let survivors: Vec<GlyphRef> = page
                .per_char
                .iter()
                .enumerate()
                .filter(|(i, _)| !deleted_here.contains(i))
                .filter_map(|(_, g)| *g)
                .collect();
            collect_deleted_codes(
                &content,
                page,
                &survivors,
                &font_tounicode,
                &mut surviving_codes,
            );
        }

        for page in &pages {
            let dels: Vec<&Detection> = detections.iter().filter(|d| d.page == page.page).collect();
            if dels.is_empty() {
                continue;
            }

            // Collect glyph byte ranges to delete, grouped by the exact string
            // they live in: (op, operand, item-within-TJ-array). Remember each
            // deleted glyph's font too, so its raw code can be scrubbed from the
            // font's `/ToUnicode`.
            let mut to_delete: Deletions = BTreeMap::new();
            let mut deleted_glyphs: Vec<GlyphRef> = Vec::new();
            for d in dels {
                for ch in d.start..d.end {
                    if let Some(Some(g)) = page.per_char.get(ch) {
                        to_delete
                            .entry(g.site)
                            .or_default()
                            .push((g.byte_start, g.byte_end));
                        deleted_glyphs.push(*g);
                    }
                }
            }
            if to_delete.is_empty() {
                continue;
            }

            // Map this page's font names to their `/ToUnicode` stream ids (only
            // fonts that have one), so a deleted glyph's code is filed under the
            // exact CMap object to scrub.
            let font_tounicode = tounicode::page_font_cmaps(&doc, page.page_id);

            let content_data = doc.get_page_content(page.page_id);
            let mut content = Content::decode(&content_data)
                .map_err(|e| Error::invalid_document(format!("decode page content: {e}")))?;
            // Before mutating, record each deleted glyph's raw code bytes against
            // its font's CMap, read straight from the string operand it lives in.
            collect_deleted_codes(
                &content,
                page,
                &deleted_glyphs,
                &font_tounicode,
                &mut deleted_codes,
            );
            apply_deletions(&mut content, &to_delete);
            let new_content = content
                .encode()
                .map_err(|e| Error::invalid_document(format!("encode page content: {e}")))?;
            doc.change_page_content(page.page_id, new_content)
                .map_err(|e| Error::invalid_document(format!("write page content: {e}")))?;
        }

        // Scrub the deleted codes from each affected font's `/ToUnicode` CMap
        // (sparing any code still used by surviving text), so the removed text
        // can't be recovered through the code->Unicode table.
        tounicode::scrub(&mut doc, &deleted_codes, &surviving_codes)?;

        sanitize::sanitize(&mut doc);

        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|e| Error::invalid_document(format!("save redacted PDF: {e}")))?;
        Ok(out)
    }

    /// Build the per-page text and per-character glyph map by walking each
    /// page's content in operator order, the same order lopdf's text
    /// extraction uses, so a character offset maps to the glyph that drew it.
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
            // A stable index per font name (the `GlyphRef::font` table), so a
            // deleted glyph can later be traced to the font that drew it.
            let font_names: Vec<Vec<u8>> = encodings.keys().cloned().collect();
            let font_index: BTreeMap<&[u8], u16> = font_names
                .iter()
                .enumerate()
                .map(|(i, name)| (name.as_slice(), i as u16))
                .collect();

            let content_data = self.doc.get_page_content(page_id);
            let content = Content::decode(&content_data)
                .map_err(|e| Error::invalid_document(format!("page {page} content: {e}")))?;

            let mut text = String::new();
            let mut per_char: Vec<Option<GlyphRef>> = Vec::new();
            let mut run = TextRun {
                text: &mut text,
                per_char: &mut per_char,
                font: 0,
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
                            // Point the run at this font's table slot (an unknown
                            // name keeps the previous slot; such text fails closed
                            // at the show operator anyway).
                            if let Some(&idx) = font_index.get(name.as_slice()) {
                                run.font = idx;
                            }
                        }
                    }
                    // The four text-showing operators (PDF 32000-1 §9.4.3). All
                    // draw glyphs and so must be decoded for redaction; they
                    // differ only in which operands carry the string(s):
                    // - `Tj`/`TJ`/`'` show from their whole operand list;
                    // - `"` (`aw ac string`) shows only its third operand, the
                    //   first two set spacing and draw nothing.
                    "Tj" | "TJ" | "'" | "\"" => match current {
                        Some(Some(enc)) => {
                            // `"` shows only its third operand; the recorded site
                            // must still address it as operand 2.
                            let (operands, base): (&[Object], usize) = if op.operator == "\"" {
                                match op.operands.get(2) {
                                    Some(s) => (std::slice::from_ref(s), 2),
                                    None => (&[], 0),
                                }
                            } else {
                                (&op.operands, 0)
                            };
                            run.show_text(enc, operands, op_idx, base)?;
                        }
                        // Text drawn under a font we could not decode or resolve:
                        // fail closed, its glyphs cannot be located for
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
                fonts: font_names,
            });
        }
        Ok(out)
    }
}

/// Read the raw glyph-code bytes of each deleted glyph out of the (still
/// unmutated) decoded content, and record them against the object id of the
/// glyph's font `/ToUnicode` CMap in `out`. Glyphs whose font has no CMap are
/// skipped. These codes drive the `/ToUnicode` scrub.
fn collect_deleted_codes(
    content: &Content,
    page: &PageText,
    deleted: &[GlyphRef],
    font_tounicode: &BTreeMap<Vec<u8>, ObjectId>,
    out: &mut BTreeMap<ObjectId, BTreeSet<Vec<u8>>>,
) {
    for g in deleted {
        let Some(name) = page.fonts.get(g.font as usize) else {
            continue;
        };
        let Some(&cmap_id) = font_tounicode.get(name) else {
            continue;
        };
        let Some(op) = content.operations.get(g.site.op) else {
            continue;
        };
        // Resolve the string operand the glyph lives in (plain `Tj` string, or
        // an element of a `TJ` array).
        let bytes: Option<&Vec<u8>> = match (op.operands.get(g.site.operand), g.site.item) {
            (Some(Object::String(b, _)), None) => Some(b),
            (Some(Object::Array(arr)), Some(k)) => match arr.get(k) {
                Some(Object::String(b, _)) => Some(b),
                _ => None,
            },
            _ => None,
        };
        if let Some(b) = bytes
            && g.byte_end <= b.len()
            && g.byte_start < g.byte_end
        {
            out.entry(cmap_id)
                .or_default()
                .insert(b[g.byte_start..g.byte_end].to_vec());
        }
    }
}

/// Remove the marked glyph byte ranges from each exact string, high-to-low so
/// earlier offsets stay valid.
///
/// Deleting glyph bytes alone is not enough to redact text: a `TJ` array
/// interleaves strings with numeric position adjustments (kerns, in thousandths
/// of an em), and those numbers, plus the advances of the removed glyphs, still
/// encode where the deleted text sat and how wide it was, enough to reconstruct
/// it (the sub-pixel glyph-position leak). So when a deletion empties a `TJ`
/// string element, the adjacent numeric adjustments are zeroed too, destroying
/// the positional residue. The surviving text reflows (there are no font metrics
/// here to preserve exact layout); a caller needing pixel-faithful layout uses
/// the raster path instead.
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
        let Some(bytes) = target else {
            continue;
        };
        // The same glyph range can be collected more than once (a ligature
        // whose one code spans several detected characters, or overlapping
        // detections). Merge overlapping/adjacent ranges so each byte span
        // is drained exactly once, draining a span twice would corrupt the
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

        // If this emptied a `TJ` string element, zero the numeric position
        // adjustments around it so no positional residue of the removed glyphs
        // survives. Re-borrow the array (the `bytes` borrow has ended) to reach
        // the sibling numbers.
        if let (Some(Object::Array(arr)), Some(k)) = (op.operands.get_mut(operand), item)
            && matches!(arr.get(k), Some(Object::String(b, _)) if b.is_empty())
        {
            neutralize_kern_neighbors(arr, k);
        }
    }
}

/// Zero the numeric position adjustments contiguous to `arr[k]` (an emptied
/// `TJ` string element) on each side, up to the next string element.
///
/// The numbers are zeroed in place rather than removed: other [`Deletions`]
/// entries for the same operation address array elements by index, so removing
/// an element would invalidate those indices. A zero adjustment draws nothing
/// and carries no position information.
fn neutralize_kern_neighbors(arr: &mut [Object], k: usize) {
    let is_kern = |o: &Object| matches!(o, Object::Integer(_) | Object::Real(_));
    let zero = |o: &mut Object| {
        *o = match o {
            Object::Real(_) => Object::Real(0.0),
            _ => Object::Integer(0),
        };
    };
    // Walk left from k until a non-numeric element (the previous string).
    for i in (0..k).rev() {
        if is_kern(&arr[i]) {
            zero(&mut arr[i]);
        } else {
            break;
        }
    }
    // Walk right from k until a non-numeric element (the next string).
    for elem in &mut arr[k + 1..] {
        if is_kern(elem) {
            zero(elem);
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use lopdf::content::{Content, Operation};

    use super::*;

    /// A single `TJ` operation whose array is `elems`.
    fn tj(elems: Vec<Object>) -> Content {
        Content {
            operations: vec![Operation::new("TJ", vec![Object::Array(elems)])],
        }
    }

    /// The array of the first `TJ` operation.
    fn tj_array(content: &Content) -> &[Object] {
        match content.operations[0].operands.first() {
            Some(Object::Array(a)) => a,
            _ => panic!("not a TJ array"),
        }
    }

    /// Deleting every glyph of a `TJ` string element empties it AND zeroes the
    /// numeric position adjustments on both sides, so no positional residue of
    /// the removed glyphs survives. The neighbours of the untouched string keep
    /// their values.
    #[test]
    fn emptying_a_tj_string_zeroes_its_kern_neighbors() {
        // [(Sec) -40 (ret) -55 ( Agent)] — delete "ret" (item 2, bytes 0..3).
        let mut content = tj(vec![
            Object::string_literal("Sec"),
            (-40).into(),
            Object::string_literal("ret"),
            (-55).into(),
            Object::string_literal(" Agent"),
        ]);
        let mut to_delete: Deletions = BTreeMap::new();
        to_delete.insert(
            GlyphSite {
                op: 0,
                operand: 0,
                item: Some(2),
            },
            vec![(0, 3)],
        );
        apply_deletions(&mut content, &to_delete);

        let arr = tj_array(&content);
        // "ret" is now empty; both adjacent kerns (-40 before, -55 after) zeroed.
        assert_eq!(arr[0], Object::string_literal("Sec"));
        assert_eq!(arr[1], Object::Integer(0), "left kern not zeroed");
        assert!(matches!(&arr[2], Object::String(b, _) if b.is_empty()));
        assert_eq!(arr[3], Object::Integer(0), "right kern not zeroed");
        assert_eq!(arr[4], Object::string_literal(" Agent"));
    }

    /// A partial deletion (some glyphs left in the string) shortens the string
    /// but does not touch the surrounding kerns: the survivors still advance, so
    /// their adjustments stay meaningful.
    #[test]
    fn a_partial_deletion_leaves_the_kerns() {
        // [(Secret) -40 (Agent)] — delete "Sec" (bytes 0..3 of item 0).
        let mut content = tj(vec![
            Object::string_literal("Secret"),
            (-40).into(),
            Object::string_literal("Agent"),
        ]);
        let mut to_delete: Deletions = BTreeMap::new();
        to_delete.insert(
            GlyphSite {
                op: 0,
                operand: 0,
                item: Some(0),
            },
            vec![(0, 3)],
        );
        apply_deletions(&mut content, &to_delete);

        let arr = tj_array(&content);
        assert_eq!(arr[0], Object::string_literal("ret"), "survivors mangled");
        assert_eq!(arr[1], Object::Integer(-40), "kern wrongly zeroed");
        assert_eq!(arr[2], Object::string_literal("Agent"));
    }

    /// Zeroing stops at the neighbouring string: a kern next to a different,
    /// still-populated string is not zeroed.
    #[test]
    fn zeroing_stops_at_the_next_string() {
        // [(a) 5 (bb) 9 (c)] — empty the middle "bb"; the 5 and 9 flank it and
        // are zeroed, but nothing beyond the flanking strings is touched.
        let mut content = tj(vec![
            Object::string_literal("a"),
            5.into(),
            Object::string_literal("bb"),
            9.into(),
            Object::string_literal("c"),
        ]);
        let mut to_delete: Deletions = BTreeMap::new();
        to_delete.insert(
            GlyphSite {
                op: 0,
                operand: 0,
                item: Some(2),
            },
            vec![(0, 2)],
        );
        apply_deletions(&mut content, &to_delete);

        let arr = tj_array(&content);
        assert_eq!(arr[0], Object::string_literal("a"));
        assert_eq!(arr[1], Object::Integer(0));
        assert_eq!(arr[3], Object::Integer(0));
        assert_eq!(arr[4], Object::string_literal("c"));
    }
}
