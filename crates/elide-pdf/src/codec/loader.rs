//! The PDF [`DocumentLoader`]: rebuild the document, read its page text,
//! surface its embedded images and attachments as blobs, and its scanned pages
//! as image streams.

#[cfg(feature = "render")]
use std::collections::BTreeSet;

use elide_codec::content::ContentData;
use elide_codec::{Blob, Document, DocumentLoader, LocalId};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::FORMAT_ID;
#[cfg(feature = "render")]
use super::SCAN_DPI;
use super::blob_id::BlobId;
use super::document::PdfDocument;
#[cfg(feature = "render")]
use super::page::PageStream;
use super::state::{Pages, PdfState};
use super::stream::PdfStream;
use crate::content::attachment::Attachments;
use crate::content::image::Images;
use crate::content::text::Text;
use crate::pdf::Pdf;
#[cfg(feature = "render")]
use crate::pdf::render::{Observation, Renders};
#[cfg(feature = "render")]
use crate::primitive::RasterMode;
use crate::sanitize::{Rebuild, Rebuilt};

/// A [`DocumentLoader`] that decodes a PDF into a [`Document`]: the page-text
/// body stream, (feature `render`) an image stream per scanned page, and a
/// [`Blob`] per embedded image and attachment.
#[derive(Debug, Default)]
pub(crate) struct PdfDocumentLoader {
    /// Whether and how pages are rendered.
    #[cfg(feature = "render")]
    raster: RasterMode,
}

impl PdfDocumentLoader {
    /// A loader under `raster`.
    #[cfg(feature = "render")]
    pub(crate) fn new(raster: RasterMode) -> Self {
        Self { raster }
    }

    /// The [`Document`] of the rebuilt `pdf`, its decoded `state`, its
    /// scanned `pages`, and its `blobs`.
    fn document(
        pdf: Pdf,
        state: PdfState,
        #[cfg(feature = "render")] pages: Vec<PageStream>,
        blobs: Vec<Blob>,
    ) -> Document {
        let surfaced = blobs
            .iter()
            .map(|blob| (blob.id.as_str().to_owned(), blob.bytes.clone()))
            .collect();
        let native = PdfDocument {
            body: PdfStream { state },
            #[cfg(feature = "render")]
            pages,
            pdf,
            surfaced,
        };
        Document::new(FORMAT_ID, native, blobs)
    }
}

#[async_trait::async_trait]
impl DocumentLoader for PdfDocumentLoader {
    async fn decode(&self, content: ContentData, _policy: HiddenContentPolicy) -> Result<Document> {
        // Hidden content goes first, so everything below (the text, the
        // rendering, the surfaced images) reads only what will ship.
        let Rebuilt { pdf, stripped } = Pdf::open(content.as_bytes())?.read(Rebuild)?;

        // Every embedded image and attached file is content of its own format,
        // decoded and redacted through the registry; one no codec reads is
        // removed on encode.
        let mut blobs: Vec<Blob> = pdf
            .read(Images)?
            .into_iter()
            .map(|image| Blob {
                id: LocalId::new(BlobId::Image(image.id).to_string()),
                bytes: image.bytes,
                hint: image.hint.to_owned(),
            })
            .collect();
        blobs.extend(pdf.read(Attachments)?.into_iter().map(|attachment| Blob {
            id: LocalId::new(BlobId::Attachment(attachment.id).to_string()),
            hint: attachment_hint(&attachment.name),
            bytes: attachment.bytes,
        }));

        #[cfg(feature = "render")]
        let (pages, scanned) = self.pages(&pdf)?;
        #[cfg(not(feature = "render"))]
        let pages = Pages::Glyph(pdf.read(Text)?);
        let state = PdfState::new(pages);
        #[cfg(feature = "render")]
        let document = Self::document(pdf, state, scanned, blobs);
        #[cfg(not(feature = "render"))]
        let document = Self::document(pdf, state, blobs);
        Ok(document.with_stripped(stripped))
    }
}

#[cfg(feature = "render")]
impl PdfDocumentLoader {
    /// The pages of `pdf` as the raster mode reads them, and each scanned
    /// page's render as an image stream, for OCR.
    ///
    /// Under [`RasterMode::Always`] the text and glyph boxes come from PDFium,
    /// and every page is replaced by a redacted image of it on encode;
    /// otherwise they come from the content walk, for glyph deletion, and
    /// under [`RasterMode::Auto`] a scanned page is rendered at [`SCAN_DPI`].
    fn pages(&self, pdf: &Pdf) -> Result<(Pages, Vec<PageStream>)> {
        let (pages, scanned, dpi) = match self.raster {
            RasterMode::Always { dpi } => {
                let pages = pdf.read(Observation)?;
                let scanned = scanned(pages.iter().map(|p| (p.page, p.text.as_str())));
                let scale = dpi.scale_factor();
                (Pages::Flatten { pages, scale }, scanned, dpi)
            }
            RasterMode::Auto => {
                let blocks = pdf.read(Text)?;
                // A page is scanned when it shows no text: text located but
                // placed nowhere (a pattern's it never paints) is no text a
                // reader sees on the page.
                let scanned = blocks
                    .iter()
                    .filter(|block| !block.shows_text())
                    .map(|block| block.page)
                    .collect();
                (Pages::Glyph(blocks), scanned, SCAN_DPI)
            }
            RasterMode::Never => return Ok((Pages::Glyph(pdf.read(Text)?), Vec::new())),
        };
        if scanned.is_empty() {
            return Ok((pages, Vec::new()));
        }
        let renders = pdf
            .read(Renders {
                pages: scanned,
                scale: dpi.scale_factor(),
            })?
            .into_iter()
            .map(|(number, png)| PageStream::new(number, png))
            .collect();
        Ok((pages, renders))
    }
}

/// The numbers of the pages with no text: scanned, or drawn as outlines.
#[cfg(feature = "render")]
fn scanned<'a>(texts: impl Iterator<Item = (u32, &'a str)>) -> BTreeSet<u32> {
    texts
        .filter(|(_, text)| text.trim().is_empty())
        .map(|(number, _)| number)
        .collect()
}

/// The registry hint for an attachment named `name`: its lowercased file
/// extension, empty when it has none (no codec then reads it, so it is removed).
fn attachment_hint(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The scanned-page stream path, built directly with a pre-rendered page (as
/// the loader's render step would produce), so it needs no PDFium.
#[cfg(all(test, feature = "render"))]
mod tests {
    use std::io::Cursor;

    use bytes::Bytes;
    use elide_codec::{Blob, Document, LocalId};
    use elide_core::ErrorKind;
    use elide_core::modality::{ChunkReader, DataWriter};
    use elide_core::redaction::Redactions;
    use elide_image::modality::{Image, ImageLocation, ImageReplacement};
    use elide_image::primitive::{BoundingBox, Dimensions, Point};
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage, load_from_memory};
    use lopdf::{Document as PdfDocument, Stream, dictionary};

    use super::PdfDocumentLoader;
    use crate::codec::blob_id::BlobId;
    use crate::codec::page::PageStream;
    use crate::codec::state::{Pages, PdfState};
    use crate::content::image::Images;
    use crate::fixtures::{DocBuilder, Page};
    use crate::pdf::Pdf;
    use crate::sanitize::Rebuild;

    /// A solid-colour PNG of `w`x`h`.
    fn png(w: u32, h: u32, colour: [u8; 3]) -> Vec<u8> {
        let image = RgbImage::from_pixel(w, h, Rgb(colour));
        let mut out = Vec::new();
        DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    /// A one-page textless 100x100pt PDF whose content stream carries a marker,
    /// to tell whether the original page content survives a reflatten.
    fn scanned_doc() -> Bytes {
        let mut pdf = DocBuilder::with_version("1.5");
        pdf.page(Page::new().content(MARKER.to_vec()).size(100, 100));
        Bytes::from(pdf.bytes())
    }

    /// The glyph-deletion document of `document`, page 1 scanned with the
    /// PNG `render`, and `blobs`.
    fn scanned_document(document: &[u8], render: &[u8], blobs: Vec<Blob>) -> Document {
        let state = PdfState::new(Pages::Glyph(Vec::new()));
        let page = PageStream::new(1, Bytes::copy_from_slice(render));
        let pdf = Pdf::open(document).unwrap().read(Rebuild).unwrap().pdf;
        PdfDocumentLoader::document(pdf, state, vec![page], blobs)
    }

    /// The scanned page's own content: a drawing the rebuild keeps.
    const MARKER: &[u8] = b"0.271828 g 0 0 1 1 re f";

    /// Whether any stream of the PDF `bytes` still holds [`MARKER`]'s drawing.
    fn holds_marker(bytes: &[u8]) -> bool {
        let doc = PdfDocument::load_mem(bytes).unwrap();
        doc.objects
            .values()
            .filter_map(|o| o.as_stream().ok())
            .any(|stream| {
                let content = stream
                    .decompressed_content()
                    .unwrap_or(stream.content.clone());
                String::from_utf8_lossy(&content).contains("0.271828")
            })
    }

    /// The region from `(x, y)` of `w`x`h` pixels.
    fn region(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(w, h),
        ))
    }

    /// Write `replacement` over `location` of page 1 of `doc`.
    async fn redact_page(
        doc: &mut Document,
        location: ImageLocation,
        replacement: ImageReplacement,
    ) -> elide_core::Result<()> {
        let mut redactions = Redactions::new();
        redactions.push(location, replacement);
        doc.stream_mut::<Image>("page-1")
            .expect("page 1 is an image stream")
            .write_at(redactions)
            .await
    }

    #[tokio::test]
    async fn a_scanned_page_is_reflattened_at_its_own_size() {
        // Rendered at 3x: the raster is 300px, the page stays 100pt.
        let mut doc = scanned_document(&scanned_doc(), &png(300, 300, [200, 200, 200]), vec![]);
        redact_page(
            &mut doc,
            region(0.0, 0.0, 300.0, 300.0),
            ImageReplacement::block(),
        )
        .await
        .unwrap();

        let out = doc.encode().unwrap().to_bytes();
        assert!(
            !holds_marker(&out),
            "original scanned page content survived reflatten"
        );
        let reopened = PdfDocument::load_mem(&out).unwrap();
        let page = *reopened.get_pages().get(&1).unwrap();
        let media_box: Vec<f32> = reopened
            .get_dictionary(page)
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_float().unwrap())
            .collect();
        assert_eq!(media_box, [0.0, 0.0, 100.0, 100.0]);
    }

    /// A redaction is painted into the render, so the page reads back
    /// redacted.
    #[tokio::test]
    async fn a_redacted_page_reads_back_redacted() {
        let mut doc = scanned_document(&scanned_doc(), &png(30, 30, [200, 200, 200]), vec![]);
        redact_page(
            &mut doc,
            region(0.0, 0.0, 30.0, 30.0),
            ImageReplacement::block(),
        )
        .await
        .unwrap();
        let page = doc.stream_mut::<Image>("page-1").unwrap();
        let chunk = page.chunks().unwrap().remove(0);
        assert!(
            chunk
                .data
                .image()
                .clone()
                .into_rgb()
                .iter()
                .all(|&s| s == 0)
        );
    }

    /// A region outside the render refuses the batch: nothing is recorded,
    /// so the page is left as it is rather than half-redacted.
    #[tokio::test]
    async fn a_region_outside_the_render_is_refused() {
        let mut doc = scanned_document(&scanned_doc(), &png(30, 30, [200, 200, 200]), vec![]);
        let err = redact_page(
            &mut doc,
            region(40.0, 40.0, 5.0, 5.0),
            ImageReplacement::block(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        let out = doc.encode().unwrap().to_bytes();
        assert!(holds_marker(&out));
    }

    /// A region addressed to another page refuses the batch: this page's
    /// render is left as it is.
    #[tokio::test]
    async fn a_region_of_another_page_is_refused() {
        let mut doc = scanned_document(&scanned_doc(), &png(30, 30, [200, 200, 200]), vec![]);
        let mut elsewhere = region(0.0, 0.0, 30.0, 30.0);
        elsewhere.page = Some(2);
        let err = redact_page(&mut doc, elsewhere, ImageReplacement::block())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
        let out = doc.encode().unwrap().to_bytes();
        assert!(holds_marker(&out));
    }

    /// A scanned page is a stream of the document, not one of its blobs.
    #[test]
    fn a_scanned_page_is_not_a_blob() {
        let mut doc = scanned_document(&scanned_doc(), &png(30, 30, [200, 200, 200]), vec![]);
        let err = doc
            .replace_blob(&LocalId::new("page-1"), Bytes::from_static(b"x"))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    /// A scanned page drawing one 2x2 gray image over the whole page, and its
    /// image surfaced as the pipeline reads it.
    fn scanned_with_image() -> (Bytes, Blob) {
        let mut pdf = DocBuilder::with_version("1.5");
        let image = pdf.add(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 2,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            vec![200; 4],
        ));
        pdf.page(
            Page::new()
                .xobject("Im1", image)
                .content(b"q 100 0 0 100 0 0 cm /Im1 Do Q".to_vec())
                .size(100, 100),
        );
        let bytes = Bytes::from(pdf.bytes());
        // Surfaced from the rebuild the loader reads, whose ids it uses.
        let surfaced = Pdf::open(&bytes)
            .unwrap()
            .read(Rebuild)
            .unwrap()
            .pdf
            .read(Images)
            .unwrap()
            .remove(0);
        let image = Blob {
            id: LocalId::new(BlobId::Image(surfaced.id).to_string()),
            bytes: surfaced.bytes,
            hint: surfaced.hint.to_owned(),
        };
        (bytes, image)
    }

    /// A redacted image on a scanned page nothing was redacted on stays
    /// redacted: the page is not replaced by its original render.
    #[test]
    fn an_unredacted_page_keeps_its_redacted_image() {
        let (bytes, image) = scanned_with_image();
        let render = png(100, 100, [200, 200, 200]);
        let mut doc = scanned_document(&bytes, &render, vec![image.clone()]);
        doc.replace_blob(&image.id, Bytes::from(png(2, 2, [0, 0, 0])))
            .unwrap();

        let out = doc.encode().unwrap().to_bytes();
        let images = Pdf::open(&out).unwrap().read(Images).unwrap();
        assert_eq!(images.len(), 1, "the page was reflattened");
        let pixels = load_from_memory(&images[0].bytes).unwrap().into_rgb8();
        assert!(
            pixels.pixels().all(|p| p.0 == [0, 0, 0]),
            "original pixels returned"
        );
    }
}
