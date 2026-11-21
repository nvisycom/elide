//! A PDF's text: every page decoded into logical text plus the [`OffsetMap`]
//! that ties each character back to the glyph bytes that drew it ([`Text`]),
//! and the glyphs of a detected span deleted through that map
//! ([`TextRedaction`]).
//!
//! The walker decodes a page's content and the forms and tiling patterns it
//! draws, through the fonts [`pdf::reader::font`](crate::pdf::reader::font)
//! reads, into a [`TextBlock`]. An [`Address`] pins where a glyph's bytes live
//! in a content stream, and an [`OffsetMap`] of [`OffsetRun`]s ties decoded
//! characters to the [`GlyphBytes`] at those addresses. Deleting a glyph also
//! scrubs its code from its font's `/ToUnicode`, unless surviving text still
//! draws it.

mod read;
mod redact;

use std::ops::Range;

use elide_core::Result;
use lopdf::ObjectId;

use crate::pdf::reader::{GlyphRect, PageFrame, PageGeometry};
use crate::pdf::{Edit, Pdf, Read, Redact};

/// Every page's text, with the map from each character back to the glyph
/// that drew it and that glyph's box.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) or
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) for a
/// page whose content cannot be read, and
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) for text that
/// cannot be located: a page that cannot be read is never treated as one
/// with nothing to redact.
pub(crate) struct Text;

impl Read for Text {
    type Output = Vec<TextBlock>;

    fn read(self, pdf: &Pdf) -> Result<Self::Output> {
        read::text_blocks(pdf)
    }
}

/// The glyphs of each detected span deleted, located through `blocks`, the
/// [`Text`] of the unedited `source`, and the codes they leave unused
/// scrubbed from their fonts' `/ToUnicode`.
///
/// A glyph is located by its byte range in the stream it was read from, so
/// this is the first change to every stream it rewrites: one an earlier
/// redaction changed is refused.
///
/// # Errors
///
/// [`ErrorKind::Redaction`](elide_core::ErrorKind::Redaction) if a detected
/// glyph cannot be found, or its stream changed since `source`, or
/// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if a
/// content stream cannot be rewritten.
pub(crate) struct TextRedaction<'a> {
    pub(crate) source: &'a Pdf,
    pub(crate) blocks: &'a [TextBlock],
    pub(crate) detections: &'a [Detection],
}

impl Redact for TextRedaction<'_> {
    fn redact(self, edit: &mut Edit) -> Result<()> {
        if self.detections.is_empty() {
            return Ok(());
        }
        redact::delete(self.source, self.blocks, edit, self.detections)
    }
}

/// One page's recovered text, plus the correspondence back to the glyphs that
/// drew it.
///
/// `text` is the decoded page text a caller runs detection over (character
/// offsets); `offsets` maps a detected character range to the glyph bytes to
/// delete. `page_id` addresses the page object for content get/set, and `fonts`
/// carries the per-glyph font table the `/ToUnicode` scrub needs. Text drawn
/// through a `Do`-invoked Form XObject is included, so a detected span is located
/// and redactable wherever it was drawn.
#[derive(Debug, Clone)]
pub(crate) struct TextBlock {
    /// 1-based page number.
    pub(crate) page: u32,
    /// The page's indirect-object id, addressing the page object for content
    /// get/set (distinct from the 1-based `page` number).
    pub(crate) page_id: ObjectId,
    /// The decoded page text, in character order.
    pub(crate) text: String,
    /// The decoded-character-to-glyph-bytes correspondence for `text`.
    pub(crate) offsets: OffsetMap,
    /// Per-glyph device-space boxes (char range → box), in draw order, for
    /// resolving a detected char span to its rendered page rectangles.
    pub(crate) rects: Vec<GlyphRect>,
    /// The page's visible rectangle and rotation, for normalizing a device box
    /// to page fractions.
    pub(crate) frame: PageFrame,
    /// The fonts referenced by the glyphs in `offsets`, indexed by
    /// [`GlyphBytes::font`](super::GlyphBytes); each carries the font's
    /// `/ToUnicode` CMap object id (when it has one) so a deleted glyph's code can
    /// be scrubbed from the right CMap even when a font name recurs across
    /// resource scopes.
    pub(crate) fonts: Vec<FontEntry>,
    /// Whether the page draws text whose glyphs can't be placed: in a font
    /// whose advances can't be known (one with no widths that names no
    /// standard face and embeds no program they are read from, or a Type3
    /// font whose matrix can't be used), or written vertically, which is
    /// not laid out. Where its glyphs land, and so what they read as and
    /// where a deletion leaves a gap, isn't known, so such a page is redacted
    /// from its render instead.
    pub(crate) unplaced: bool,
}

impl TextBlock {
    /// The page's glyph boxes and frame.
    pub(crate) fn geometry(&self) -> PageGeometry<'_> {
        PageGeometry {
            rects: &self.rects,
            frame: &self.frame,
        }
    }

    /// Whether the page shows text: a glyph placed on it stands for a
    /// character that is not whitespace. Text located but placed nowhere (a
    /// pattern's that it never paints) shows nothing.
    #[cfg(any(feature = "render", test))]
    pub(crate) fn shows_text(&self) -> bool {
        let chars: Vec<char> = self.text.chars().collect();
        self.rects.iter().any(|glyph| {
            chars
                .get(glyph.chars.clone())
                .is_some_and(|shown| shown.iter().any(|c| !c.is_whitespace()))
        })
    }
}

/// A font referenced while walking content, resolved to its `/ToUnicode` CMap.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FontEntry {
    /// The font's `/ToUnicode` stream object id, or `None` when the font has no
    /// CMap (nothing to scrub).
    pub(crate) to_unicode: Option<ObjectId>,
}

/// A detected span to redact: a character range into a page's text.
///
/// The offsets are Unicode scalar (`char`) offsets into the page text, the
/// same text and unit that [`Text`](crate::content::text::Text) and the rendered
/// [`Observation`](crate::pdf::render::Observation) both produce. One
/// `Detection` type serves every redaction path: the text-layer rewrite
/// resolves the range to glyph bytes through the page's
/// [`OffsetMap`](crate::content::text::OffsetMap), and the raster path resolves the
/// same range to glyph pixel boxes. A caller never picks a coordinate unit per
/// path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Detection {
    /// 1-based page number the span is on.
    pub(crate) page: u32,
    /// Start character offset into the page text (inclusive).
    pub(crate) start: usize,
    /// End character offset into the page text (exclusive).
    pub(crate) end: usize,
}

impl Detection {
    /// A detection of `[start, end)` (character offsets) on `page`.
    pub(crate) fn new(page: u32, start: usize, end: usize) -> Self {
        Self { page, start, end }
    }
}

/// Which content stream a glyph lives in: the page's own content, or the content
/// of a Form XObject the page (or another XObject) draws with `Do`.
///
/// A glyph's byte offsets are relative to the operand string within *this*
/// stream, and a deletion edits *this* stream. Distinguishing the streams lets
/// text drawn through a `Do`-invoked Form XObject be located and redacted like
/// page-level text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum StreamTarget {
    /// The page's concatenated content stream.
    PageContent,
    /// A Form XObject's content stream, addressed by its object id.
    XObject(ObjectId),
}

/// The address of the string that draws a glyph: which content stream, which
/// operation within it, which operand of that operation, and which string
/// *within* that operand (an element of a `TJ` array, or the whole `Tj` string).
///
/// Ordered so it can key a `BTreeMap` of pending edits; the byte range within
/// the addressed string is carried separately (see [`OffsetRun`](super::OffsetRun)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Address {
    /// The stream the glyph's operator lives in.
    pub(crate) stream: StreamTarget,
    /// Index of the operation within the decoded content stream.
    pub(crate) op: usize,
    /// Index of the operand within that operation.
    pub(crate) operand: usize,
    /// Index of the string within a `TJ` array operand, or `None` for a plain
    /// `Tj`/`'`/`"` string operand.
    pub(crate) item: Option<usize>,
}

/// One stretch of the decoded-to-glyph-bytes correspondence: a contiguous range
/// of decoded characters and the glyph bytes, at one [`Address`], that drew
/// them.
///
/// `chars` is the page-local decoded character range this run covers.
/// `source` is `Some((address, byte_start, byte_end))` for characters a glyph
/// drew, where the byte range is within that address's string operand, or `None`
/// for a synthetic gap (a word space between runs, which no glyph drew and which
/// a redaction leaves alone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OffsetRun {
    /// The page-local decoded character range this run covers.
    pub(crate) chars: Range<usize>,
    /// The glyph bytes that drew those characters, or `None` for a gap.
    pub(crate) source: Option<GlyphBytes>,
}

/// The glyph bytes that drew a run's characters: the string operand address,
/// the byte range within that operand's string, the code they spell, and the
/// font that drew it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GlyphBytes {
    /// Which stream/operation/operand/item the glyph's code lives in.
    pub(crate) address: Address,
    /// Start byte offset of the glyph's code within that string operand.
    pub(crate) byte_start: usize,
    /// End byte offset (exclusive) of the glyph's code within that operand.
    pub(crate) byte_end: usize,
    /// The glyph's code: its bytes, big-endian.
    pub(crate) code: u32,
    /// Index into the block's font table of the font that drew the glyph, so its
    /// code can be scrubbed from that font's `/ToUnicode` CMap.
    pub(crate) font: u16,
}

/// The decoded-text-to-glyph-bytes correspondence for one page, as a list of
/// [`runs`](OffsetRun) contiguous in the decoded-character dimension.
///
/// [`glyph_bytes`](Self::glyph_bytes) maps a detected character range to the
/// [`GlyphBytes`] it must delete, from which the redactor groups byte ranges by
/// [`Address`] and collects the glyph codes to scrub from `/ToUnicode`. A glyph
/// whose one code drew several characters (a ligature) appears in several runs
/// sharing the same [`GlyphBytes`], so callers coalesce by address + byte range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OffsetMap {
    runs: Vec<OffsetRun>,
}

impl OffsetMap {
    /// Build a map from an explicit list of runs.
    pub(crate) fn new(runs: Vec<OffsetRun>) -> Self {
        Self { runs }
    }

    /// The map's runs, in decoded-character order.
    pub(crate) fn runs(&self) -> &[OffsetRun] {
        &self.runs
    }

    /// The glyph bytes drawing the characters in `chars`, in decoded order.
    ///
    /// Yields one [`GlyphBytes`] per glyph the range covers; gaps contribute
    /// nothing. A ligature glyph spanning several characters yields its shared
    /// [`GlyphBytes`] once per covered character, callers coalesce by address +
    /// byte range (a byte span drained twice would corrupt the string).
    pub(crate) fn glyph_bytes(&self, chars: Range<usize>) -> impl Iterator<Item = GlyphBytes> + '_ {
        self.runs
            .iter()
            .filter(move |run| {
                chars.start < chars.end
                    && run.chars.start < chars.end
                    && run.chars.end > chars.start
            })
            .filter_map(|run| run.source)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    use super::*;
    use crate::fixtures::{DocBuilder, Page, to_unicode_cmap, type1};
    use crate::pdf::reader::GlyphRect;
    use crate::sanitize::{Check, Rebuild};

    /// Every stream of the PDF `bytes`, decoded, as text.
    fn streams_of(bytes: &[u8]) -> String {
        let doc = Document::load_mem(bytes).unwrap();
        doc.objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .map(|stream| {
                String::from_utf8_lossy(
                    &stream
                        .decompressed_content()
                        .unwrap_or(stream.content.clone()),
                )
                .into_owned()
            })
            .collect()
    }

    /// An empty range covers no character, so it selects no glyph, even
    /// inside a ligature's characters.
    #[test]
    fn an_empty_range_selects_no_glyph() {
        let ligature = GlyphBytes {
            address: Address {
                stream: StreamTarget::PageContent,
                op: 0,
                operand: 0,
                item: None,
            },
            byte_start: 0,
            byte_end: 1,
            code: 0,
            font: 0,
        };
        let map = OffsetMap::new(vec![OffsetRun {
            chars: 3..5,
            source: Some(ligature),
        }]);
        assert_eq!(map.glyph_bytes(4..4).count(), 0);
        assert_eq!(map.glyph_bytes(4..5).count(), 1);
    }

    /// Text a tiling pattern draws is detected and deleted like the page's:
    /// the pattern's one stream draws every tile.
    #[test]
    fn text_in_a_tiling_pattern_is_found_and_deleted() {
        tiling_text(Object::Integer(1), 1);
    }

    /// A tiling pattern whose `/PatternType` is the real `1.0` is still a
    /// tiling pattern, and its text is still found.
    #[test]
    fn a_real_pattern_type_is_still_a_tiling_pattern() {
        tiling_text(Object::Real(1.0), 1);
    }

    /// A pattern selected before every fill, more times than the page's walk
    /// budget, is walked once: its text reads once and the page is not
    /// refused.
    #[test]
    fn a_pattern_selected_many_times_is_walked_once() {
        tiling_text(Object::Integer(1), 5_000);
    }

    /// The glyph boxes of the text a tiling pattern draws, its page painting
    /// with it as `content` draws.
    fn tiled_rects(content: &[u8]) -> Vec<GlyphRect> {
        tiled_block(content).rects
    }

    /// The text block of a page painting with a tiling pattern as `content`
    /// draws.
    fn tiled_block(content: &[u8]) -> TextBlock {
        let bytes = tiling_document(Object::Integer(1), content.to_vec());
        let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
        pdf.read(Text).unwrap().remove(0)
    }

    /// A pattern's text shows on every tile it paints: its glyph boxes are
    /// placed on each, from its cell (50 by 20 points) out.
    #[test]
    fn pattern_text_is_placed_on_each_tile_painted() {
        let one = tiled_rects(b"/Pattern cs /P0 scn 0 0 50 20 re f");
        assert!(!one.is_empty());
        let four = tiled_rects(b"/Pattern cs /P0 scn 0 0 100 40 re f");
        assert_eq!(four.len(), 4 * one.len());
        assert!(
            four.iter()
                .any(|glyph| glyph.rect.min_x >= 50.0 && glyph.rect.min_y > 15.0),
            "no box on the far tile: {four:?}"
        );
    }

    /// A region painted twice places its tiles once, so painting over the
    /// same tiles again is not charged again.
    #[test]
    fn tiles_painted_twice_are_placed_once() {
        let once = tiled_rects(b"/Pattern cs /P0 scn 0 0 100 40 re f");
        let twice = tiled_rects(b"/Pattern cs /P0 scn 0 0 100 40 re f 0 0 100 40 re f");
        assert_eq!(once.len(), twice.len());
    }

    /// Tiles far past where a float places them exactly, or more than the
    /// page's budget, refuse the page rather than place without bound.
    #[test]
    fn tiles_past_the_budget_are_refused() {
        for content in [
            &b"/Pattern cs /P0 scn -1000000000000000000000.0 0 2000000000000000000000.0 20 re f"[..],
            b"/Pattern cs /P0 scn 0 0 1000000 1000000 re f",
        ] {
            let bytes = tiling_document(Object::Integer(1), content.to_vec());
            let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
            let error = pdf.read(Text).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::ResourceLimit);
        }
    }

    /// A pattern selected but never painted shows its text nowhere, so its
    /// page shows no text, though the text is located for redaction.
    #[test]
    fn an_unpainted_pattern_places_no_box() {
        assert!(tiled_rects(b"/Pattern cs /P0 scn").is_empty());
        let block = tiled_block(b"/Pattern cs /P0 scn");
        assert!(block.text.contains("Jane Roe") && !block.shows_text());
        assert!(tiled_block(b"/Pattern cs /P0 scn 0 0 50 20 re f").shows_text());
    }

    /// A clip bounds what a fill paints: a fill of four tiles clipped to one
    /// places the pattern's text on that one.
    #[test]
    fn a_clip_bounds_the_tiles_painted() {
        let one = tiled_rects(b"/Pattern cs /P0 scn 0 0 50 20 re f");
        let clipped = tiled_rects(b"/Pattern cs /P0 scn 0 0 50 20 re W n 0 0 100 40 re f");
        assert_eq!(clipped.len(), one.len());
        let restored = tiled_rects(
            b"q /Pattern cs /P0 scn 0 0 50 20 re W n Q /Pattern cs /P0 scn 0 0 100 40 re f",
        );
        assert_eq!(restored.len(), 4 * one.len());
    }

    /// Pattern text shows on a tile only where it is painted: a fill of the
    /// tile's top edge shows none of the text on its baseline, and one of
    /// its right edge only the glyphs that reach it.
    #[test]
    fn a_tile_shows_only_where_it_is_painted() {
        assert!(tiled_rects(b"/Pattern cs /P0 scn 0 15 50 5 re f").is_empty());
        let right = tiled_rects(b"/Pattern cs /P0 scn 40 0 10 20 re f");
        let all = tiled_rects(b"/Pattern cs /P0 scn 0 0 50 20 re f");
        assert!(!right.is_empty() && right.len() < all.len(), "{right:?}");
        assert!(
            right.iter().all(|glyph| glyph.rect.max_x > 40.0),
            "{right:?}"
        );
    }

    /// Text paints with a pattern as its rendering mode draws it: filled,
    /// stroked, or, invisible, not at all; and nothing paints at zero
    /// alpha.
    #[test]
    fn text_and_alpha_paint_as_drawn() {
        let shown = |mode: &str| {
            tiled_rects(
                format!(
                    "BT /F2 12 Tf 0 5 Td {mode} Tr /Pattern cs /P0 scn /Pattern CS /P0 SCN \
                     (xxxxx) Tj ET"
                )
                .as_bytes(),
            )
            .len()
        };
        let text = tiled_block(b"BT /F2 12 Tf 0 5 Td (xxxxx) Tj ET")
            .rects
            .len();
        assert!(shown("0") > text && shown("1") > text);
        assert_eq!(shown("3"), text);
        assert!(tiled_rects(b"/Clear gs /Pattern cs /P0 scn 0 0 50 20 re f").is_empty());
    }

    /// A glyph the clip hides has no box, though its text is read; text
    /// shown under a clipping mode clips what follows its text object.
    #[test]
    fn clips_bound_glyphs_and_text_clips_what_follows() {
        let clipped = tiled_block(b"0 0 1 1 re W n BT /F2 12 Tf 50 50 Td (xx) Tj ET");
        assert!(clipped.text.contains("xx") && clipped.rects.is_empty());
        let after_text_clip =
            tiled_rects(b"BT /F2 12 Tf 0 50 Td 7 Tr (x) Tj ET /Pattern cs /P0 scn 0 0 50 20 re f");
        assert_eq!(
            after_text_clip
                .iter()
                .filter(|g| g.rect.min_y < 40.0)
                .count(),
            0
        );
    }

    /// A stencil mask paints its unit square with the fill pattern.
    #[test]
    fn a_stencil_mask_paints_with_the_pattern() {
        let one = tiled_rects(b"/Pattern cs /P0 scn 0 0 50 20 re f");
        let stencil = tiled_rects(b"q 50 0 0 20 0 0 cm /Pattern cs /P0 scn /Sm Do Q");
        assert_eq!(stencil.len(), one.len());
    }

    /// A pattern drawing no text needs no tiles, however tiny they are.
    #[test]
    fn a_pattern_of_no_text_is_not_tiled() {
        tiled_block(b"/Pattern cs /G0 scn 0 0 100 100 re f");
    }

    /// A stroke paints past its path by its width: a line along a row
    /// boundary paints the tiles on either side.
    #[test]
    fn a_stroke_paints_its_width() {
        assert!(!tiled_rects(b"/Pattern CS /P0 SCN 4 w 1 j 0 20 m 100 20 l S").is_empty());
    }

    /// A form fills with the pattern its caller selected; one selecting the
    /// pattern itself, drawn at two places, tiles each on its own grid; and
    /// a pattern filling another's cell shows on each of that one's tiles.
    #[test]
    fn patterns_follow_forms_and_other_patterns() {
        let one = tiled_rects(b"/Pattern cs /P0 scn 0 0 50 20 re f");
        assert_eq!(tiled_rects(b"/Pattern cs /P0 scn /Fm Do").len(), one.len());
        // A fill a form's `/BBox` clips away paints nothing.
        assert!(tiled_rects(b"/Pattern cs /P0 scn /Fo Do").is_empty());

        let twice = tiled_rects(b"q /Fs Do Q q 1 0 0 1 25 0 cm /Fs Do Q");
        assert_eq!(twice.len(), 2 * one.len());
        let start = one
            .iter()
            .map(|glyph| glyph.rect.min_x)
            .fold(f64::INFINITY, f64::min);
        assert!(
            twice
                .iter()
                .any(|glyph| (glyph.rect.min_x - (start + 25.0)).abs() < 1e-6),
            "no box on the second form's grid: {twice:?}"
        );

        let nested = tiled_rects(b"/Pattern cs /Q0 scn 0 0 100 20 re f");
        assert_eq!(nested.len(), 2 * one.len());
        assert!(nested.iter().any(|glyph| glyph.rect.min_x >= 50.0));
    }

    /// Selecting a pattern paints nothing, so its cell's text is read apart
    /// from the text the page shows around the selection, never inside it.
    #[test]
    fn a_selected_pattern_reads_apart_from_the_text_around_it() {
        let bytes = tiling_document(
            Object::Integer(1),
            b"BT /F1 12 Tf 10 50 Td (alice@) Tj /Pattern cs /P0 scn (example.com) Tj ET".to_vec(),
        );
        let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
        let text = &pdf.read(Text).unwrap()[0].text;
        assert_eq!(text, "alice@example.com Jane Roe ");
    }

    /// A pattern that draws no text adds no gap: the text around its
    /// selection reads as one.
    #[test]
    fn a_pattern_drawing_no_text_leaves_no_gap() {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        let pattern = pdf.stream(
            dictionary! {
                "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
                "BBox" => vec![0.into(), 0.into(), 10.into(), 10.into()],
                "XStep" => 10, "YStep" => 10,
            },
            b"0 0 5 5 re f".to_vec(),
        );
        pdf.page(
            Page::new()
                .font("F1", font)
                .resource("Pattern", "P0", pattern)
                .content(b"BT /F1 12 Tf 10 50 Td (ali) Tj /Pattern cs /P0 scn (ce) Tj ET".to_vec())
                .size(100, 100),
        );
        let bytes = pdf.bytes();
        let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
        assert_eq!(pdf.read(Text).unwrap()[0].text, "alice");
    }

    /// Find and delete the text a tiling pattern of `/PatternType` `kind`
    /// draws, the page selecting it `selections` times.
    fn tiling_text(kind: Object, selections: usize) {
        let bytes = tiling_document(
            kind,
            b"/Pattern cs /P0 scn 0 0 100 100 re f\n".repeat(selections),
        );
        let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
        let blocks = pdf.read(Text).unwrap();
        assert_eq!(blocks[0].text.trim_end(), "Jane Roe");
        let mut edit = pdf.edit();
        edit.redact(TextRedaction {
            source: &pdf,
            blocks: &blocks,
            detections: &[Detection::new(1, 0, 8)],
        })
        .unwrap();
        let out = edit.finish().unwrap();
        assert!(!streams_of(&out.bytes()).contains("Jane Roe"));
        out.read(Check).unwrap();
    }

    /// Text a soft mask's group draws shows wherever the mask lets later
    /// painting through, so it is read and deleted like the page's own.
    #[test]
    fn text_in_a_soft_mask_group_is_found_and_deleted() {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        let group = pdf.form(
            dictionary! {
                "BBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
                "Group" => dictionary! { "S" => "Transparency", "CS" => "DeviceGray" },
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            b"BT /F1 12 Tf 10 100 Td (Jane Roe) Tj ET".to_vec(),
        );
        let state = pdf.add(dictionary! {
            "Type" => "ExtGState",
            "SMask" => dictionary! { "Type" => "Mask", "S" => "Luminosity", "G" => group },
        });
        pdf.page(
            Page::new()
                .resource("ExtGState", "GS1", state)
                .content(b"/GS1 gs 0 0 200 200 re f".to_vec())
                .size(200, 200),
        );
        let pdf = Pdf::open(&pdf.bytes()).unwrap().read(Rebuild).unwrap().pdf;
        let blocks = pdf.read(Text).unwrap();
        assert_eq!(blocks[0].text.trim(), "Jane Roe");
        let mut edit = pdf.edit();
        edit.redact(TextRedaction {
            source: &pdf,
            blocks: &blocks,
            detections: &[Detection::new(1, 0, 8)],
        })
        .unwrap();
        let out = edit.finish().unwrap();
        assert!(!streams_of(&out.bytes()).contains("Jane Roe"));
        out.read(Check).unwrap();
    }

    /// A soft mask's text is read apart from the page's: a value the page
    /// draws across the state that sets the mask stays whole.
    #[test]
    fn soft_mask_text_falls_outside_the_page_text() {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        let group = pdf.form(
            dictionary! {
                "BBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
                "Group" => dictionary! { "S" => "Transparency", "CS" => "DeviceGray" },
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            b"BT /F1 12 Tf 10 50 Td (Jane Roe) Tj ET".to_vec(),
        );
        let state = pdf.add(dictionary! {
            "Type" => "ExtGState",
            "SMask" => dictionary! { "Type" => "Mask", "S" => "Luminosity", "G" => group },
        });
        pdf.page(
            Page::new()
                .font("F1", font)
                .resource("ExtGState", "GS1", state)
                .content(b"BT /F1 12 Tf 10 100 Td (alice@) Tj /GS1 gs (example.com) Tj ET".to_vec())
                .size(200, 200),
        );
        let pdf = Pdf::open(&pdf.bytes()).unwrap().read(Rebuild).unwrap().pdf;
        let text = &pdf.read(Text).unwrap()[0].text;
        assert!(text.contains("alice@example.com"), "{text:?}");
        assert!(text.contains("Jane Roe"), "{text:?}");
    }

    /// A soft mask a tiling pattern's cell sets draws in that cell, so its
    /// text shows on every tile the pattern paints.
    #[test]
    fn a_soft_mask_set_in_a_pattern_shows_on_its_tiles() {
        let rects = |area: &[u8]| {
            let mut pdf = DocBuilder::with_version("1.5");
            let font = pdf.add(type1("Helvetica"));
            let group = pdf.form(
                dictionary! {
                    "BBox" => vec![0.into(), 0.into(), 50.into(), 20.into()],
                    "Group" => dictionary! { "S" => "Transparency", "CS" => "DeviceGray" },
                    "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                },
                b"BT /F1 8 Tf 2 5 Td (Jane) Tj ET".to_vec(),
            );
            let state = pdf.add(dictionary! {
                "Type" => "ExtGState",
                "SMask" => dictionary! { "Type" => "Mask", "S" => "Luminosity", "G" => group },
            });
            let pattern = pdf.stream(
                dictionary! {
                    "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
                    "BBox" => vec![0.into(), 0.into(), 50.into(), 20.into()],
                    "XStep" => 50, "YStep" => 20,
                    "Resources" => dictionary! { "ExtGState" => dictionary! { "GS1" => state } },
                },
                b"/GS1 gs 0 0 50 20 re f".to_vec(),
            );
            pdf.page(
                Page::new()
                    .resource("Pattern", "P0", pattern)
                    .content([b"/Pattern cs /P0 scn ".as_slice(), area].concat())
                    .size(200, 200),
            );
            let pdf = Pdf::open(&pdf.bytes()).unwrap().read(Rebuild).unwrap().pdf;
            pdf.read(Text).unwrap().remove(0).rects
        };
        let one = rects(b"0 0 50 20 re f");
        assert!(!one.is_empty());
        let four = rects(b"0 0 100 40 re f");
        assert_eq!(four.len(), 4 * one.len());
    }

    /// A one-page document drawing `content`, its `/F1` Helvetica, its `/P0`
    /// a tiling pattern of `/PatternType` `kind` showing "Jane Roe", its
    /// `/Q0` a pattern of the same cell filled with `/P0`, its `/Fm` a form
    /// filling that cell with whatever it is drawn with, and its `/Fs` a form
    /// filling it with `/P0`, and its `/Fo` a form filling outside its `/BBox`;
    /// its `/F2` a Helvetica with widths, its `/Sm` a stencil mask, its `/G0`
    /// a pattern of tiny tiles drawing no text, and its `/Clear` a graphics
    /// state painting nothing.
    fn tiling_document(kind: Object, content: Vec<u8>) -> Vec<u8> {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        let cell = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("Jane Roe")]),
                Operation::new("ET", vec![]),
            ],
        };
        let pattern = pdf.add(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => kind, "PaintType" => 1, "TilingType" => 1,
                "BBox" => vec![0.into(), 0.into(), 50.into(), 20.into()],
                "XStep" => 50, "YStep" => 20,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            cell.encode().unwrap(),
        ));
        let cell_box = || vec![0.into(), 0.into(), 50.into(), 20.into()];
        let filled = pdf.add(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
                "BBox" => cell_box(), "XStep" => 50, "YStep" => 20,
                "Resources" => dictionary! { "Pattern" => dictionary! { "P0" => pattern } },
            },
            b"/Pattern cs /P0 scn 0 0 50 20 re f".to_vec(),
        ));
        let form = |pdf: &mut DocBuilder, content: &[u8]| {
            pdf.add(Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form", "BBox" => cell_box(),
                    "Resources" => dictionary! { "Pattern" => dictionary! { "P0" => pattern } },
                },
                content.to_vec(),
            ))
        };
        let fills = form(&mut pdf, b"0 0 50 20 re f");
        let selects = form(&mut pdf, b"/Pattern cs /P0 scn 0 0 50 20 re f");
        let outside = form(&mut pdf, b"0 40 50 20 re f");
        let wide = pdf.add(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
            "FirstChar" => 32, "LastChar" => 126,
            "Widths" => vec![Object::Integer(500); 95],
        });
        let stencil = pdf.add(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 1, "Height" => 1,
                "ImageMask" => true,
            },
            vec![0],
        ));
        let hatch = pdf.add(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => 1, "PaintType" => 1, "TilingType" => 1,
                "BBox" => vec![0.into(), 0.into(), Object::Real(0.01), Object::Real(0.01)],
                "XStep" => Object::Real(0.01), "YStep" => Object::Real(0.01),
                "Resources" => dictionary! {},
            },
            b"0 0 0.005 0.005 re f".to_vec(),
        ));
        pdf.page(
            Page::new()
                .resources(dictionary! {
                    "Font" => dictionary! { "F1" => font, "F2" => wide },
                    "Pattern" => dictionary! { "P0" => pattern, "Q0" => filled, "G0" => hatch },
                    "XObject" => dictionary! {
                        "Fm" => fills, "Fs" => selects, "Fo" => outside, "Sm" => stencil,
                    },
                    "ExtGState" => dictionary! { "Clear" => dictionary! { "ca" => 0, "CA" => 0 } },
                })
                .content(content)
                .size(100, 100),
        );
        pdf.bytes()
    }

    /// Deleting the only glyph a font's `/ToUnicode` maps leaves a CMap of
    /// no code, which the output still reads and passes the check with.
    #[test]
    fn deleting_the_only_mapped_glyph_leaves_a_readable_cmap() {
        let mut pdf = DocBuilder::with_version("1.5");
        let cmap = pdf.add(Stream::new(
            dictionary! {},
            to_unicode_cmap(&["<00> <FF>"], &["<4A> <004A>"], &[]),
        ));
        let mut font = type1("Helvetica");
        font.set("ToUnicode", cmap);
        let font = pdf.add(font);
        pdf.page(
            Page::new()
                .font("F1", font)
                .content(b"BT /F1 12 Tf (J) Tj ET".to_vec())
                .size(100, 100),
        );
        let bytes = pdf.bytes();
        let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
        let blocks = pdf.read(Text).unwrap();
        assert_eq!(blocks[0].text, "J");
        let mut edit = pdf.edit();
        edit.redact(TextRedaction {
            source: &pdf,
            blocks: &blocks,
            detections: &[Detection::new(1, 0, 1)],
        })
        .unwrap();
        let out = edit.finish().unwrap();
        out.read(Check).unwrap();
        assert!(out.read(Text).unwrap()[0].text.is_empty());
        let doc = out.doc();
        let cmap = doc
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .find_map(|font| font.get(b"ToUnicode").ok())
            .and_then(|cmap| doc.get_object(cmap.as_reference().ok()?).ok())
            .and_then(|cmap| cmap.as_stream().ok())
            .and_then(|cmap| cmap.decompressed_content().ok())
            .map(|cmap| String::from_utf8_lossy(&cmap).to_ascii_uppercase())
            .unwrap();
        assert!(cmap.contains("BEGINCODESPACERANGE"), "{cmap}");
        assert!(!cmap.contains("<4A>"), "{cmap}");
    }

    /// A stream another redaction changed no longer holds the glyphs where the
    /// text was read, so deleting from it is refused rather than writing the
    /// read bytes back over the change.
    #[test]
    fn a_stream_changed_since_its_text_was_read_is_refused() {
        let mut pdf = DocBuilder::with_version("1.5");
        let font = pdf.add(type1("Helvetica"));
        pdf.page(
            Page::new()
                .font("F1", font)
                .content(b"BT /F1 12 Tf (Jane Roe) Tj ET".to_vec())
                .size(100, 100),
        );
        let (doc, pages) = pdf.finish();
        let page = pages[0];
        let pdf = Edit::of(doc).finish().unwrap();
        let blocks = pdf.read(Text).unwrap();

        let mut edit = pdf.edit();
        edit.doc_mut()
            .change_page_content(page, b"BT /F1 12 Tf (Jane) Tj ET".to_vec())
            .unwrap();
        let error = edit
            .redact(TextRedaction {
                source: &pdf,
                blocks: &blocks,
                detections: &[Detection::new(1, 0, 4)],
            })
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Redaction);
    }
}
