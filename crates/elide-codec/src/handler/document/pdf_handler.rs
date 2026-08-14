//! PDF handler: adapts the standalone [`elide_pdf`] engine to the codec's
//! [`Handler`] contract.
//!
//! Each page's born-digital text is streamed as one [`Chunk`]; a redaction on
//! that chunk records a `(find, replace)` edit for the page, and [`encode`]
//! applies them through [`Pdf::rewrite`](elide_pdf::Pdf::rewrite), which owns the PDF round-trip.
//! Fail-closed: if a targeted text can't be located on its page, the rewrite is
//! refused rather than emitting a document where it survives.
//!
//! Unlike DOCX, a PDF is *not* a zip and its text is not byte-addressable, so
//! redaction targets `(page, text)` rather than a byte span.
//!
//! Under the `pdf-render` feature and [`OcrMode::Force`], pages are also
//! rasterised on decode and surface as image parts for the OCR pipeline.
//!
//! [`encode`]: Handler::encode
//! [`OcrMode::Force`]: super::OcrMode::Force

use bytes::Bytes;
#[cfg(feature = "pdf-render")]
use elide_core::modality::image::ImageData;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::operator::Redactions;
use elide_core::{Error, ErrorKind, Result};

#[cfg(feature = "pdf-render")]
use super::OcrMode;
use super::PdfLoader;
use crate::codec::{Container, Part, PartId};
use crate::content::ContentData;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the PDF codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pdf");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// Decodes on the born-digital text path (no page rendering). To rasterise
/// pages for OCR, build the format with [`format_with`] instead.
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PdfLoader::new())
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// [`Format`] descriptor that rasterises PDF pages for OCR per `ocr`.
///
/// Mirrors the force-OCR switch other tools expose (OCRmyPDF `--force-ocr`,
/// Docling `force_full_page_ocr`): under [`OcrMode::Force`] every page is
/// rendered to an image on decode for the image/OCR pipeline.
///
/// [`OcrMode::Force`]: super::OcrMode::Force
#[cfg(feature = "pdf-render")]
#[cfg_attr(docsrs, doc(cfg(feature = "pdf-render")))]
pub fn format_with(ocr: OcrMode) -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PdfLoader::with_ocr(ocr))
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// One page's text and where it sits in the concatenated text stream.
#[derive(Debug, Clone)]
pub(crate) struct PdfPage {
    /// 1-based page number.
    pub(crate) number: u32,
    /// The page's current (possibly redacted) text.
    pub(crate) text: String,
    /// Start offset of this page in the concatenated stream.
    pub(crate) start: usize,
}

/// PDF text handler backed by [`elide_pdf`].
///
/// Streams each page's text as a chunk, records per-page redactions, and on
/// [`encode`](Handler::encode) rewrites the born-digital text layer.
#[derive(Debug, Default)]
pub(crate) struct PdfHandler {
    /// The original document bytes, retained so [`elide_pdf`] rewrites and
    /// re-serialises from the true source.
    pub(crate) document: Bytes,
    /// Extracted pages, in page order, with stream offsets for `read_next`.
    pub(crate) pages: Vec<PdfPage>,
    /// Read cursor over `pages`.
    pub(crate) cursor: usize,
    /// Recorded text edits: `(page, find, replace)`, applied at encode.
    pub(crate) edits: Vec<elide_pdf::block::Replacement>,
    /// Pages rasterised for OCR, present only under the `pdf-render` feature
    /// and [`OcrMode::Force`]; empty on the text path.
    ///
    /// [`OcrMode::Force`]: super::OcrMode::Force
    #[cfg(feature = "pdf-render")]
    pub(crate) rendered: Vec<ImageData>,
}

impl PdfHandler {
    /// A text handler over the extracted `pages` of `document`.
    pub(crate) fn text(document: Bytes, pages: Vec<PdfPage>) -> Self {
        Self {
            document,
            pages,
            cursor: 0,
            edits: Vec::new(),
            #[cfg(feature = "pdf-render")]
            rendered: Vec::new(),
        }
    }

    /// A handler carrying pages rasterised for OCR.
    #[cfg(feature = "pdf-render")]
    pub(crate) fn rendered(document: Bytes, rendered: Vec<ImageData>) -> Self {
        Self {
            document,
            rendered,
            ..Self::default()
        }
    }

    /// The page whose stream range contains `offset`, and the offset within it.
    fn page_at(&self, offset: usize) -> Option<(&PdfPage, usize)> {
        self.pages
            .iter()
            .find(|p| offset >= p.start && offset < p.start + p.text.len())
            .map(|p| (p, offset - p.start))
    }
}

#[async_trait::async_trait]
impl Handler<Text> for PdfHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        if self.edits.is_empty() {
            return Ok(ContentData::new(self.document.clone()));
        }
        let out = elide_pdf::Pdf::open(&self.document)
            .and_then(|pdf| pdf.rewrite(&self.edits))
            .map_err(pdf_error)?;
        Ok(ContentData::new(Bytes::from(out)))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        if self.cursor >= self.pages.len() {
            return Ok(None);
        }
        let page = &self.pages[self.cursor];
        let chunk = Chunk {
            location: TextLocation {
                start: page.start,
                end: page.start + page.text.len(),
                page: Some(page.number),
            },
            data: TextData::new(page.text.clone()),
            hints: Vec::new(),
        };
        self.cursor += 1;
        Ok(Some(chunk))
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        let base = chunk.location.start;
        let start = base + local.start;
        let end = base + local.end;
        if start > end || end > chunk.location.end {
            return None;
        }
        Some(TextLocation {
            start,
            end,
            page: chunk.location.page,
        })
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        Some(self)
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for PdfHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some((page, local)) = self.page_at(location.start) else {
            return Ok(None);
        };
        let local_end = location.end - page.start;
        Ok(page.text.get(local..local_end).map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for PdfHandler {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        for (location, replacement) in redactions.into_iter() {
            let Some((page, local)) = self.page_at(location.start) else {
                continue;
            };
            let local_end = location.end - page.start;
            let Some(find) = page.text.get(local..local_end) else {
                continue;
            };
            self.edits.push(elide_pdf::block::Replacement::new(
                page.number,
                find.to_owned(),
                // `Removed` replaces with nothing.
                replacement.value().unwrap_or_default().to_owned(),
            ));
        }
        Ok(())
    }
}

impl Container for PdfHandler {
    fn parts(&self) -> Vec<Part> {
        // Pages rendered for OCR (under `pdf-render` and `OcrMode::Force`)
        // surface as image parts for the OCR pipeline. On the text path there
        // are none: born-digital text is handled directly, and native XObject
        // image extraction is still to come.
        #[cfg(feature = "pdf-render")]
        {
            self.rendered
                .iter()
                .enumerate()
                .map(|(index, page)| Part {
                    id: PartId::from(format!("page-{index}")),
                    bytes: page.bytes.clone(),
                    hint: "png".to_string(),
                })
                .collect()
        }
        #[cfg(not(feature = "pdf-render"))]
        Vec::new()
    }

    fn replace_part(&mut self, id: &PartId, _bytes: Bytes) -> Result<()> {
        // Rendered pages are detection-only inputs; folding a redacted page
        // image back into the PDF is a raster-rewrite path not supported here.
        Err(Error::new(
            ErrorKind::CapabilityUnavailable,
            format!("pdf replace_part: `{id}` is not a writable part"),
        ))
    }
}

/// Map an [`elide_pdf`] error into the codec's error type.
pub(super) fn pdf_error(err: elide_pdf::Error) -> Error {
    use elide_pdf::ErrorKind as PdfKind;
    let kind = match err.kind() {
        PdfKind::InvalidDocument | PdfKind::LimitExceeded => ErrorKind::MalformedInput,
        PdfKind::UnsafeRewrite => ErrorKind::Processing,
        _ => ErrorKind::Processing,
    };
    Error::new(kind, err.to_string())
}
