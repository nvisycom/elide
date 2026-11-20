//! The PDF [`DocumentLoader`]: rebuild the document, read its page text,
//! surface its embedded images and attachments as blobs, and its scanned pages
//! as image streams.

#[cfg(feature = "render")]
use std::collections::BTreeSet;

use elide_codec::content::ContentData;
use elide_codec::{Blob, Document, DocumentLoader, LocalId};
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};

use super::FORMAT_ID;
#[cfg(feature = "render")]
use super::SCAN_DPI;
use super::blob_id::BlobId;
use super::document::PdfDocument;
#[cfg(feature = "render")]
use super::page::PageStream;
use super::state::PdfState;
use super::stream::PdfStream;
use crate::content::attachment::Attachments;
use crate::content::image::Images;
use crate::content::text::{Text, TextBlock};
use crate::pdf::Pdf;
#[cfg(feature = "render")]
use crate::pdf::render::Renders;
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
    /// Metadata and collaboration content are stripped whatever `_policy`
    /// asks: the rebuild models neither as content.
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
        let (state, rendered) = self.pages(&pdf)?;
        #[cfg(not(feature = "render"))]
        let state = PdfState::new(placed(pdf.read(Text)?)?);
        #[cfg(feature = "render")]
        let document = Self::document(pdf, state, rendered, blobs);
        #[cfg(not(feature = "render"))]
        let document = Self::document(pdf, state, blobs);
        Ok(document.with_stripped(stripped))
    }
}

#[cfg(feature = "render")]
impl PdfDocumentLoader {
    /// The state of the pages of `pdf` redacted by deleting glyphs, and the
    /// render of each page redacted from it, for OCR.
    ///
    /// Under [`RasterMode::Always`] every page is redacted from its render
    /// at its resolution: detected on what it shows, and replaced by it; its
    /// text layer is not read, so one that can't be decoded doesn't stop it.
    /// Under [`RasterMode::Auto`] a page that draws text whose glyphs can't
    /// be placed is too, at [`SCAN_DPI`], and a scanned page is rendered for
    /// OCR and replaced only where something on it is redacted.
    /// Under [`RasterMode::Never`] nothing is rendered, so a page that would
    /// need it is refused.
    ///
    /// # Errors
    ///
    /// As [`Text`] outside [`RasterMode::Always`], as [`placed`] under
    /// [`RasterMode::Never`], and as [`Renders`].
    fn pages(&self, pdf: &Pdf) -> Result<(PdfState, Vec<PageStream>)> {
        let (state, scanned, replaced, dpi) = match self.raster {
            RasterMode::Never => {
                return Ok((PdfState::new(placed(pdf.read(Text)?)?), Vec::new()));
            }
            RasterMode::Always { dpi } => {
                let every: BTreeSet<u32> = pdf.doc().get_pages().into_keys().collect();
                (PdfState::new(Vec::new()), BTreeSet::new(), every, dpi)
            }
            RasterMode::Auto => {
                let (unplaced, placed): (Vec<TextBlock>, Vec<TextBlock>) = pdf
                    .read(Text)?
                    .into_iter()
                    .partition(|block| block.unplaced);
                // A page is scanned when it shows no text: text located but
                // placed nowhere (a pattern's it never paints) is no text a
                // reader sees on the page.
                let scanned = placed
                    .iter()
                    .filter(|block| !block.shows_text())
                    .map(|block| block.page)
                    .collect();
                let replaced = unplaced.iter().map(|block| block.page).collect();
                (PdfState::new(placed), scanned, replaced, SCAN_DPI)
            }
        };
        let scale = dpi.scale_factor();
        let state = state.rendered_at(scale);
        let pages: BTreeSet<u32> = scanned.union(&replaced).copied().collect();
        if pages.is_empty() {
            return Ok((state, Vec::new()));
        }
        let renders = pdf
            .read(Renders { pages, scale })?
            .into_iter()
            .map(|(number, png)| {
                let page = PageStream::new(number, png);
                if replaced.contains(&number) {
                    page.replaced()
                } else {
                    page
                }
            })
            .collect();
        Ok((state, renders))
    }
}

/// `blocks`, every page of which can be redacted by deleting its glyphs.
///
/// # Errors
///
/// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
/// for a page that draws text whose glyphs can't be placed (in a font whose
/// advances can't be known, or written vertically):
/// where its glyphs land is up to each viewer, so it can be redacted only from
/// its render, which needs the `render` feature and a raster mode other than
/// `Never`.
fn placed(blocks: Vec<TextBlock>) -> Result<Vec<TextBlock>> {
    if let Some(block) = blocks.iter().find(|block| block.unplaced) {
        return Err(Error::capability_unavailable(format!(
            "page {} draws text whose glyphs can't be placed (in a font whose widths \
             can't be known, or written vertically); it can be redacted only from its render",
            block.page
        )));
    }
    Ok(blocks)
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
    use elide_codec::content::ContentData;
    use elide_codec::{Blob, Document, DocumentLoader, LocalId};
    use elide_core::ErrorKind;
    use elide_core::modality::text::Text;
    use elide_core::modality::{ChunkReader, DataReader, DataWriter};
    use elide_core::redaction::{HiddenContentPolicy, Redactions};
    use elide_image::modality::{Image, ImageLocation, ImageReplacement};
    use elide_image::primitive::{BoundingBox, Dimensions, Point};
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage, load_from_memory};
    use lopdf::{Document as PdfDocument, Stream, dictionary};

    use super::PdfDocumentLoader;
    use crate::codec::blob_id::BlobId;
    use crate::codec::page::PageStream;
    use crate::codec::state::PdfState;
    use crate::content::image::Images;
    use crate::fixtures::{DocBuilder, FontFile, Page, Sfnt, embedding, shown, type1};
    use crate::pdf::Pdf;
    use crate::primitive::RasterMode;
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
        let page = PageStream::new(1, Bytes::copy_from_slice(render));
        rendered_document(document, page, blobs)
    }

    /// The document of `document` with page 1 redacted from `page`, and
    /// `blobs`.
    fn rendered_document(document: &[u8], page: PageStream, blobs: Vec<Blob>) -> Document {
        let state = PdfState::new(Vec::new());
        let pdf = Pdf::open(document).unwrap().read(Rebuild).unwrap().pdf;
        PdfDocumentLoader::document(pdf, state, vec![page], blobs)
    }

    /// A one-page 100x100pt PDF showing `Jane Roe` in a font that gives no
    /// widths and names no standard face.
    fn unplaced_doc() -> Bytes {
        let mut pdf = DocBuilder::new();
        let font = pdf.add(type1("Custom"));
        pdf.page(
            Page::new()
                .font("F1", font)
                .ops(shown("Jane Roe"))
                .size(100, 100),
        );
        Bytes::from(pdf.bytes())
    }

    /// `bytes` decoded under `raster`.
    async fn decoded(bytes: Bytes, raster: RasterMode) -> elide_core::Result<Document> {
        PdfDocumentLoader::new(raster)
            .decode(ContentData::new(bytes), HiddenContentPolicy::default())
            .await
    }

    /// The text of the body stream of `doc`.
    fn body_text(doc: &mut Document) -> String {
        doc.stream_mut::<Text>("body")
            .expect("the body stream")
            .chunks()
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.data.as_str().to_owned())
            .collect()
    }

    /// Read every page stream of `doc`, as detection reads it.
    fn read_pages(doc: &mut Document) {
        for number in 1.. {
            let Some(page) = doc.stream_mut::<Image>(&format!("page-{number}")) else {
                break;
            };
            page.chunks().unwrap();
        }
    }

    /// A page redacted from its render is replaced by it with nothing on it
    /// redacted, so its text layer never ships.
    #[test]
    fn a_rendered_page_is_replaced_with_nothing_redacted() {
        let render = png(100, 100, [255, 255, 255]);
        let page = PageStream::new(1, Bytes::from(render)).replaced();
        let mut doc = rendered_document(&scanned_doc(), page, vec![]);
        read_pages(&mut doc);
        assert!(!holds_marker(doc.encode().unwrap().as_bytes()));

        let kept = scanned_document(&scanned_doc(), &png(100, 100, [255, 255, 255]), vec![]);
        assert!(holds_marker(kept.encode().unwrap().as_bytes()));
    }

    /// A page replaced by its render keeps no print box: each placed the
    /// page as it was, not its image.
    #[test]
    fn a_rendered_page_keeps_no_print_box() {
        let mut pdf = DocBuilder::with_version("1.5");
        let trim = vec![10.into(), 10.into(), 90.into(), 90.into()];
        pdf.page(
            Page::new()
                .content(MARKER.to_vec())
                .size(100, 100)
                .set("TrimBox", trim),
        );
        let render = png(100, 100, [255, 255, 255]);
        let page = PageStream::new(1, Bytes::from(render)).replaced();
        let mut doc = rendered_document(&pdf.bytes(), page, vec![]);
        read_pages(&mut doc);
        let out = PdfDocument::load_mem(doc.encode().unwrap().as_bytes()).unwrap();
        let page = *out.get_pages().get(&1).unwrap();
        assert!(out.get_dictionary(page).unwrap().get(b"TrimBox").is_err());
    }

    /// A page replaced by its render that nothing read is refused: with no
    /// image pipeline to detect on it, what it shows would ship unredacted.
    #[test]
    fn an_unread_rendered_page_is_refused() {
        let render = png(100, 100, [255, 255, 255]);
        let page = PageStream::new(1, Bytes::from(render)).replaced();
        let doc = rendered_document(&scanned_doc(), page, vec![]);
        let error = doc.encode().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
    }

    /// Reading one region of a replaced page's render is not reading the
    /// page: it is refused as if nothing read it.
    #[tokio::test]
    async fn a_region_read_does_not_count_as_reading_the_page() {
        let render = png(100, 100, [255, 255, 255]);
        let page = PageStream::new(1, Bytes::from(render)).replaced();
        let mut doc = rendered_document(&scanned_doc(), page, vec![]);
        let page = doc
            .stream_mut::<Image>("page-1")
            .expect("the page is a stream");
        page.read_at(&region(0.0, 0.0, 10.0, 10.0)).await.unwrap();
        let error = doc.encode().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
    }

    /// With rendering off, a page drawing text in a font whose advances
    /// can't be known is refused rather than redacted by guesswork.
    #[tokio::test]
    async fn never_refuses_a_page_in_a_font_without_widths() {
        let Err(error) = decoded(unplaced_doc(), RasterMode::Never).await else {
            panic!("a page in a font without widths must be refused");
        };
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
    }

    /// A page drawing text in a font that gives no widths but embeds a
    /// program they are read from is read from its text, even with rendering
    /// off.
    #[tokio::test]
    async fn a_page_in_an_embedded_font_without_widths_is_read() {
        let mut pdf = DocBuilder::new();
        let program =
            Sfnt::new(1000, &[(".notdef", 0), ("A", 400), ("B", 500)]).cmap(3, 1, 0x41, &[1, 2]);
        let font = embedding(
            pdf.doc(),
            "TrueType",
            FontFile::TrueType,
            program.build(),
            32,
        );
        let font = pdf.add(font);
        pdf.page(Page::new().font("F1", font).ops(shown("AB")).size(100, 100));
        let mut doc = decoded(Bytes::from(pdf.bytes()), RasterMode::Never)
            .await
            .unwrap();
        assert_eq!(body_text(&mut doc).trim(), "AB");
    }

    /// Under the default mode a page drawing text in a font whose advances
    /// can't be known is redacted from its render: it surfaces as an image
    /// stream, its text is not read, and it is replaced on encode.
    #[tokio::test]
    #[ignore = "requires the native PDFium library at runtime"]
    async fn auto_renders_a_page_in_a_font_without_widths() {
        let mut doc = decoded(unplaced_doc(), RasterMode::Auto).await.unwrap();
        assert!(body_text(&mut doc).trim().is_empty());
        read_pages(&mut doc);
        let out = doc.encode().unwrap();
        let mut again = decoded(Bytes::copy_from_slice(out.as_bytes()), RasterMode::Never)
            .await
            .unwrap();
        assert!(
            body_text(&mut again).trim().is_empty(),
            "a text layer survived"
        );
    }

    /// Under `Always` every page is redacted from its render, its text layer
    /// never read, whatever its fonts: even one whose codes can't be decoded,
    /// which glyph deletion refuses.
    #[tokio::test]
    #[ignore = "requires the native PDFium library at runtime"]
    async fn always_renders_every_page_without_reading_its_text() {
        let mut pdf = DocBuilder::new();
        let font = pdf.add(type1("Helvetica"));
        let mut undecodable = type1("Custom");
        undecodable.set("Encoding", "Identity-H");
        let undecodable = pdf.add(undecodable);
        pdf.page(
            Page::new()
                .font("F1", font)
                .ops(shown("Jane Roe"))
                .size(100, 100),
        );
        pdf.page(
            Page::new()
                .font("F1", undecodable)
                .ops(shown("John Doe"))
                .size(100, 100),
        );
        let bytes = Bytes::from(pdf.bytes());
        assert!(decoded(bytes.clone(), RasterMode::Never).await.is_err());
        let mut doc = decoded(bytes, RasterMode::always()).await.unwrap();
        assert!(body_text(&mut doc).trim().is_empty());
        assert!(doc.stream_mut::<Image>("page-1").is_some());
        assert!(doc.stream_mut::<Image>("page-2").is_some());
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
