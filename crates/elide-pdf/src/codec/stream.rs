//! The PDF body [`Stream<Text>`]: yields each page's text as a chunk and records
//! per-page redactions onto the shared [`PdfState`].

use std::ops::Range;

use elide_codec::string::ContiguousSpan;
use elide_codec::{FormatId, Stream};
use elide_core::modality::text::{Marked, Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::primitive::UnitBoundingBox;
use elide_core::redaction::Redactions;
use elide_core::{Error, Result};

use super::FORMAT_ID;
use super::state::PdfState;

/// The whole of a page, as a rendered rectangle.
const WHOLE_PAGE: UnitBoundingBox = UnitBoundingBox::new(0.0, 0.0, 1.0, 1.0);

/// The body [`Stream<Text>`] of a PDF: yields each page's text as a chunk and
/// records per-page redactions onto the shared [`PdfState`], which the
/// [`PdfDocument`](super::document::PdfDocument) applies on encode.
pub(super) struct PdfStream {
    pub(super) state: PdfState,
}

impl Stream<Text> for PdfStream {
    fn format(&self) -> FormatId {
        FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        // One chunk per page, each addressed in its *own* page-local byte space
        // (`0..len`). A whole-page rendered anchor carries which page the chunk
        // is on (so `lift` and redaction know it) without a document-wide offset;
        // detection runs per chunk, so a span can never straddle a page boundary.
        Ok(self
            .state
            .texts()
            .into_iter()
            .map(|(number, text)| Chunk {
                location: TextLocation::rendered(0..text.len(), number, [WHOLE_PAGE]),
                data: TextData::new(text),
                hints: Vec::new(),
            })
            .collect())
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        // The chunk is already page-local, so the recognizer's range is the
        // page-local range; just validate it against the page's own bounds.
        let chunk_range = chunk.location.range()?;
        let local_range = local.range()?;
        if local_range.start > local_range.end || local_range.end > chunk_range.end {
            return None;
        }
        let number = chunk.location.page()?;

        // Resolve the entity's rendered rectangles from this page's glyph
        // geometry. The range is byte offsets, but glyph geometry is keyed by
        // character offset, so convert against the page text (which the chunk
        // carries) before looking the span up. A span covering only synthetic
        // gaps yields no boxes, so falls back to a whole-page anchor: the
        // entity is still located on its page.
        let page_text = chunk.data.as_str();
        let rects = self
            .state
            .geometry(number)
            .and_then(|geometry| {
                let char_start = page_text.get(..local_range.start)?.chars().count();
                let char_end = page_text.get(..local_range.end)?.chars().count();
                let boxes = geometry.rects_for(char_start..char_end);
                (!boxes.is_empty()).then_some(boxes)
            })
            .unwrap_or_else(|| vec![WHOLE_PAGE]);

        // PDF text has no flat source byte coordinate (glyph runs in content
        // streams), so no byte anchor is carried — only the page-local span and
        // the rendered rectangles.
        Some(TextLocation::rendered(local_range.clone(), number, rects))
    }

    /// A rendered-page mark becomes the page-local span of the glyphs it
    /// covers, keeping the reviewer's rectangles. The glyphs must form one
    /// span of the page text (see [`ContiguousSpan`]): a box over a table
    /// column covers glyphs with other text between them, and is refused
    /// rather than deleting that text too.
    fn resolve(&self, location: TextLocation) -> Result<TextLocation> {
        let TextLocation::Marked(mark) = &location else {
            return Ok(location);
        };
        let Marked::Rendered { page, rects } = mark else {
            return Err(Error::malformed(
                "a source-byte mark does not address a PDF page",
            ));
        };
        let number = *page;
        let text = self.state.text(number)?;
        let geometry = self.state.geometry(number).ok_or_else(|| {
            Error::malformed(format!(
                "page {number} has no glyph geometry to place a mark on"
            ))
        })?;
        // Glyphs address characters; the span addresses bytes.
        let bytes: Vec<usize> = text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
            .collect();
        let pieces = geometry
            .glyphs_in(rects)
            .map(
                |chars| match (bytes.get(chars.start), bytes.get(chars.end)) {
                    (Some(&start), Some(&end)) => Ok(start..end),
                    _ => Err(Error::malformed(format!(
                        "a glyph on page {number} is outside its text"
                    ))),
                },
            )
            .collect::<Result<Vec<_>>>()?;
        let span = text.contiguous_span(pieces)?;
        Ok(TextLocation::rendered(span, number, rects.clone()))
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for PdfStream {
    async fn read_at(&self, location: &TextLocation) -> Result<TextData> {
        let (range, number) = page_span(location)?;
        self.state
            .text(number)?
            .get(range.clone())
            .map(TextData::new)
            .ok_or_else(|| {
                Error::malformed(format!(
                    "range {}..{} is outside page {number}'s text or splits a character",
                    range.start, range.end
                ))
            })
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for PdfStream {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        let spans = redactions
            .into_iter()
            .map(|(location, _replacement)| {
                page_span(&location).map(|(range, page)| (page, range.clone()))
            })
            .collect::<Result<Vec<_>>>()?;
        self.state.record(spans)
    }
}

/// The page-local span and page a location addresses: PDF text is read and
/// written by both.
///
/// # Errors
///
/// [`MalformedInput`] for a location with no span (an unresolved mark) or no
/// page.
///
/// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
fn page_span(location: &TextLocation) -> Result<(&Range<usize>, u32)> {
    match (location.range(), location.page()) {
        (Some(range), Some(page)) => Ok((range, page)),
        _ => Err(Error::malformed(
            "a PDF location needs a page-local span and its page",
        )),
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::TextReplacement;
    use lopdf::content::{Content, Operation};
    use lopdf::{Document, Object, Stream as PdfObjectStream, dictionary};

    use super::*;
    use crate::codec::state::Pages;
    use crate::content::text::Text;
    use crate::fixtures::with_pages;
    use crate::pdf::Pdf;
    #[cfg(feature = "render")]
    use crate::pdf::reader::{DeviceRect, GlyphRect, PageFrame};
    #[cfg(feature = "render")]
    use crate::pdf::render::PageObservation;

    /// The body stream of a one-page document showing `Jane Roe`.
    fn body() -> PdfStream {
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Tj", vec![Object::string_literal("Jane Roe")]),
            Operation::new("ET", vec![]),
        ];
        let content = doc.add_object(PdfObjectStream::new(
            dictionary! {},
            Content { operations }.encode().unwrap(),
        ));
        with_pages(
            &mut doc,
            [dictionary! {
                "Contents" => content,
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            }],
        );
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let blocks = Pdf::open(&bytes).unwrap().read(Text).unwrap();
        PdfStream {
            state: PdfState::new(Pages::Glyph(blocks)),
        }
    }

    fn redaction(page: u32, range: Range<usize>) -> (TextLocation, TextReplacement) {
        (
            TextLocation::rendered(range, page, [WHOLE_PAGE]),
            TextReplacement::substituted("[NAME]"),
        )
    }

    /// A batch with one span the document doesn't have records none of it.
    #[tokio::test]
    async fn a_failed_batch_records_nothing() {
        let mut body = body();
        let batch = Redactions::from([redaction(1, 0..4), redaction(2, 0..4)]);
        assert!(body.write_at(batch).await.is_err());
        assert!(body.state.detections().is_empty());

        body.write_at(Redactions::from([redaction(1, 0..4)]))
            .await
            .unwrap();
        assert_eq!(body.state.detections().len(), 1);
    }

    /// An empty span is checked like any other before it is skipped: on a
    /// page the document doesn't have, or reversed, it fails the batch.
    #[tokio::test]
    async fn an_empty_span_is_checked_before_it_is_skipped() {
        let mut body = body();
        let missing = Redactions::from([redaction(1, 0..4), redaction(2, 0..0)]);
        assert!(body.write_at(missing).await.is_err());
        assert!(body.state.detections().is_empty());
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = Redactions::from([redaction(1, 0..4), redaction(1, 4..2)]);
        assert!(body.write_at(reversed).await.is_err());
        assert!(body.state.detections().is_empty());

        body.write_at(Redactions::from([redaction(1, 2..2)]))
            .await
            .unwrap();
        assert!(body.state.detections().is_empty());
    }

    /// A reviewer's mark on a flattened page resolves through PDFium's glyph
    /// boxes, placed by the page's frame.
    #[cfg(feature = "render")]
    #[test]
    fn a_mark_resolves_on_a_flattened_page() {
        // "ab cd" on a Letter page: each glyph 10pt wide on a baseline at
        // y = 700, the space between the words drawing nothing.
        let glyph = |chars: Range<usize>, x: f64| GlyphRect {
            chars,
            rect: DeviceRect {
                min_x: x,
                min_y: 700.0,
                max_x: x + 10.0,
                max_y: 710.0,
            },
        };
        let page = PageObservation {
            page: 1,
            text: "ab cd".to_owned(),
            glyphs: vec![
                glyph(0..1, 100.0),
                glyph(1..2, 110.0),
                glyph(3..4, 300.0),
                glyph(4..5, 310.0),
            ],
            frame: PageFrame::LETTER,
        };
        let body = PdfStream {
            state: PdfState::new(Pages::Flatten {
                pages: vec![page],
                scale: 1.0,
            }),
        };
        // A box over "cd": x 300..320 of 612, y 82..92 from the top of 792.
        let mark = TextLocation::marked_rendered(
            1,
            UnitBoundingBox::new(295.0 / 612.0, 80.0 / 792.0, 30.0 / 612.0, 14.0 / 792.0),
        );
        let resolved = body.resolve(mark).unwrap();
        assert_eq!(resolved.range(), Some(&(3..5)));
    }
}
