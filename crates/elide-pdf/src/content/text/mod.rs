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
}

impl TextBlock {
    /// The page's glyph boxes and frame.
    pub(crate) fn geometry(&self) -> PageGeometry<'_> {
        PageGeometry {
            rects: &self.rects,
            frame: &self.frame,
        }
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
/// the byte range within that operand's string, and the font that drew it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GlyphBytes {
    /// Which stream/operation/operand/item the glyph's code lives in.
    pub(crate) address: Address,
    /// Start byte offset of the glyph's code within that string operand.
    pub(crate) byte_start: usize,
    /// End byte offset (exclusive) of the glyph's code within that operand.
    pub(crate) byte_end: usize,
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
            .filter(move |run| run.chars.start < chars.end && run.chars.end > chars.start)
            .filter_map(|run| run.source)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream, dictionary};

    use super::*;
    use crate::fixtures::with_pages;
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

    /// Find and delete the text a tiling pattern of `/PatternType` `kind`
    /// draws, the page selecting it `selections` times.
    fn tiling_text(kind: Object, selections: usize) {
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let cell = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Tj", vec![Object::string_literal("Jane Roe")]),
                Operation::new("ET", vec![]),
            ],
        };
        let pattern = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "Pattern", "PatternType" => kind, "PaintType" => 1, "TilingType" => 1,
                "BBox" => vec![0.into(), 0.into(), 50.into(), 20.into()],
                "XStep" => 50, "YStep" => 20,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            },
            cell.encode().unwrap(),
        ));
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"/Pattern cs /P0 scn 0 0 100 100 re f\n".repeat(selections),
        ));
        with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "Resources" => dictionary! { "Pattern" => dictionary! { "P0" => pattern } },
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            }],
        );
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        let pdf = Pdf::open(&bytes).unwrap().read(Rebuild).unwrap().pdf;
        let blocks = pdf.read(Text).unwrap();
        assert_eq!(blocks[0].text, "Jane Roe");
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

    /// A stream another redaction changed no longer holds the glyphs where the
    /// text was read, so deleting from it is refused rather than writing the
    /// read bytes back over the change.
    #[test]
    fn a_stream_changed_since_its_text_was_read_is_refused() {
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 12 Tf (Jane Roe) Tj ET".to_vec(),
        ));
        let page = with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            }],
        )[0];
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
