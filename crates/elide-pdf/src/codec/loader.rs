//! The PDF [`DocumentLoader`]: sanitize the document, read its page text, and
//! surface its embedded images, attachments, and scanned pages as blobs.

#[cfg(feature = "render")]
use std::collections::BTreeSet;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{Blob, Document, DocumentLoader, LocalId};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::FORMAT_ID;
#[cfg(feature = "render")]
use super::SCAN_DPI;
use super::document::PdfDocument;
use super::part_id::PartId;
use super::state::{Pages, PdfState};
use super::stream::PdfStream;
use crate::document::Pdf;
#[cfg(feature = "render")]
use crate::primitive::RasterMode;

/// A [`DocumentLoader`] that decodes a PDF into a [`Document`]: the page-text
/// body stream, and a [`Blob`] per embedded image, attachment, and (feature
/// `render`) scanned page.
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

    /// The [`Document`] of the sanitized `document` bytes, its decoded
    /// `state`, and its `blobs`.
    fn document(document: Bytes, state: PdfState, blobs: Vec<Blob>) -> Document {
        let surfaced = blobs
            .iter()
            .map(|blob| (blob.id.as_str().to_owned(), blob.bytes.clone()))
            .collect();
        let native = PdfDocument {
            body: PdfStream { state },
            document,
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
        let (document, stripped) = Pdf::open(content.as_bytes())?.sanitize()?;
        let document = Bytes::from(document);
        let pdf = Pdf::open(&document)?;

        // Every embedded image and attached file is content of its own format,
        // decoded and redacted through the registry; one no codec reads is
        // removed on encode.
        let mut blobs: Vec<Blob> = pdf
            .images()?
            .into_iter()
            .map(|image| Blob {
                id: LocalId::new(PartId::Image(image.id).to_string()),
                bytes: image.bytes,
                hint: image.hint.to_owned(),
            })
            .collect();
        blobs.extend(pdf.attachments()?.into_iter().map(|attachment| Blob {
            id: LocalId::new(PartId::Attachment(attachment.id).to_string()),
            hint: attachment_hint(&attachment.name),
            bytes: attachment.bytes,
        }));

        let pages = self.pages(&pdf, &mut blobs)?;
        Ok(Self::document(document, PdfState::new(pages), blobs).with_stripped(stripped))
    }
}

impl PdfDocumentLoader {
    /// The pages of `pdf` as the raster mode reads them, surfacing each
    /// scanned page into `blobs` as a rendered image for OCR.
    ///
    /// Under [`RasterMode::Always`] the text and glyph boxes come from PDFium,
    /// and every page is replaced by a redacted image of it on encode;
    /// otherwise they come from the content walk, for glyph deletion, and
    /// under [`RasterMode::Auto`] a scanned page is rendered at [`SCAN_DPI`].
    #[cfg(feature = "render")]
    fn pages(&self, pdf: &Pdf, blobs: &mut Vec<Blob>) -> Result<Pages> {
        let (pages, scanned, dpi) = match self.raster {
            RasterMode::Always { dpi } => {
                let pages = pdf.observe()?;
                let scanned = scanned(pages.iter().map(|p| (p.page, p.text.as_str())));
                let scale = dpi.scale_factor();
                (Pages::Flatten { pages, scale }, scanned, dpi)
            }
            RasterMode::Auto => {
                let blocks = pdf.text()?;
                let scanned = scanned(blocks.iter().map(|b| (b.page, b.text.as_str())));
                (Pages::Glyph(blocks), scanned, SCAN_DPI)
            }
            RasterMode::Never => return Ok(Pages::Glyph(pdf.text()?)),
        };
        if scanned.is_empty() {
            return Ok(pages);
        }
        for (number, png) in pdf.render_pages(scanned, dpi.scale_factor())? {
            blobs.push(Blob {
                id: LocalId::new(PartId::Page(number).to_string()),
                bytes: png,
                hint: "png".to_owned(),
            });
        }
        Ok(pages)
    }

    /// The pages of `pdf`, read by the content walk for glyph deletion.
    #[cfg(not(feature = "render"))]
    fn pages(&self, pdf: &Pdf, _blobs: &mut Vec<Blob>) -> Result<Pages> {
        Ok(Pages::Glyph(pdf.text()?))
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

/// The scanned-page blob path, built directly with a pre-rendered page (as the
/// loader's render step would produce), so it needs no PDFium.
#[cfg(all(test, feature = "render"))]
mod tests {
    use std::io::Cursor;

    use bytes::Bytes;
    use elide_codec::{Blob, Document, LocalId};
    use elide_core::ErrorKind;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage, load_from_memory};
    use lopdf::{Document as PdfDocument, Object, Stream, dictionary};

    use super::PdfDocumentLoader;
    use crate::codec::part_id::PartId;
    use crate::codec::state::{Pages, PdfState};
    use crate::document::Pdf;

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
        let mut doc = PdfDocument::with_version("1.5");
        let pages_id = doc.new_object_id();
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"q 1 0 0 1 0 0 cm % SCANNED-PAGE-MARKER".to_vec(),
        ));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        });
        let pages = dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        Bytes::from(out)
    }

    /// The glyph-deletion document of `document`, with each scanned page's
    /// raster as a `page-N` blob.
    fn scanned_document(document: Bytes, scanned: &[(u32, Vec<u8>)]) -> Document {
        let blobs = scanned
            .iter()
            .map(|(number, png)| Blob {
                id: LocalId::new(format!("page-{number}")),
                bytes: Bytes::from(png.clone()),
                hint: "png".to_owned(),
            })
            .collect();
        let state = PdfState::new(Pages::Glyph(Vec::new()));
        PdfDocumentLoader::document(document, state, blobs)
    }

    #[test]
    fn a_scanned_page_is_reflattened_at_its_own_size() {
        // Rendered at 3x: the raster is 300px, the page stays 100pt.
        let mut doc = scanned_document(scanned_doc(), &[(1, png(300, 300, [200, 200, 200]))]);
        let redacted = Bytes::from(png(300, 300, [0, 0, 0]));
        doc.replace_blob(&LocalId::new("page-1"), redacted)
            .expect("page-1 accepted");

        let out = doc.encode().unwrap().to_bytes();
        assert!(
            !String::from_utf8_lossy(&out).contains("SCANNED-PAGE-MARKER"),
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

    #[test]
    fn rejects_a_page_id_the_document_did_not_surface() {
        let mut doc = scanned_document(scanned_doc(), &[]);
        let err = doc
            .replace_blob(&LocalId::new("page-1"), Bytes::from_static(b"x"))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    /// A scanned page drawing one 2x2 gray image, its image surfaced as the
    /// pipeline reads it, and its surfaced page render.
    fn scanned_with_image() -> (Bytes, Blob, Blob) {
        let mut doc = PdfDocument::with_version("1.5");
        let image = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 2,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            vec![200; 4],
        ));
        let content = doc.add_object(Stream::new(
            dictionary! {},
            b"q 100 0 0 100 0 0 cm /Im1 Do Q".to_vec(),
        ));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages, "Contents" => content,
            "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => image } },
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        });
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let bytes = Bytes::from(bytes);
        let surfaced = Pdf::open(&bytes).unwrap().images().unwrap().remove(0);
        let image = Blob {
            id: LocalId::new(PartId::Image(surfaced.id).to_string()),
            bytes: surfaced.bytes,
            hint: surfaced.hint.to_owned(),
        };
        let page = Blob {
            id: LocalId::new("page-1"),
            bytes: Bytes::from(png(100, 100, [200, 200, 200])),
            hint: "png".to_owned(),
        };
        (bytes, image, page)
    }

    /// A redacted image on a scanned page whose render came back unchanged
    /// stays redacted: the page is not replaced by its original render.
    #[test]
    fn an_unchanged_page_render_keeps_its_redacted_image() {
        let (bytes, image, page) = scanned_with_image();
        let state = PdfState::new(Pages::Glyph(Vec::new()));
        let mut doc = PdfDocumentLoader::document(bytes, state, vec![image.clone(), page]);
        doc.replace_blob(&image.id, Bytes::from(png(2, 2, [0, 0, 0])))
            .unwrap();

        let out = doc.encode().unwrap().to_bytes();
        let images = Pdf::open(&out).unwrap().images().unwrap();
        assert_eq!(images.len(), 1, "the page was reflattened");
        let pixels = load_from_memory(&images[0].bytes).unwrap().into_rgb8();
        assert!(
            pixels.pixels().all(|p| p.0 == [0, 0, 0]),
            "original pixels returned"
        );
    }
}
