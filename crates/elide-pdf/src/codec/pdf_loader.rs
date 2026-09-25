//! The PDF [`DocumentLoader`]: decode page text (and, on the glyph path, surface
//! redactable image / scanned-page blobs), plus the helpers that build and parse
//! the sub-part ids.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{Document, DocumentLoader, DocumentPart, ErasedStream, LocalId, Stream};
use elide_core::Result;
use elide_core::modality::text::Text;

use super::pdf_recombine::PdfRecombine;
use super::pdf_state::{PdfInner, PdfState};
use super::pdf_stream::PdfStream;
use super::{BODY_PART_ID, FORMAT_ID, PAGE_SEPARATOR, PdfPage, RedactMode};
use crate::document::Pdf;
use crate::extract::Block;
#[cfg(feature = "image")]
use crate::extract::{EmbeddingKind, ImageId};
#[cfg(feature = "render")]
use crate::primitive::RasterMode;
#[cfg(feature = "render")]
use crate::render::PageObservation;

/// A [`DocumentLoader`] that decodes a PDF into a [`Document`]: the page-text
/// body [`Stream`] plus, on the glyph-deletion path, a
/// [`Blob`](DocumentPart::Blob) per redactable embedded image and (feature
/// `render` + `image`) per textless scanned page.
#[derive(Debug, Default)]
pub(crate) struct PdfDocumentLoader {
    /// Whether redaction flattens pages to images (raster) instead of the
    /// default glyph deletion. Only meaningful with the `render` feature.
    #[cfg(feature = "render")]
    raster: RasterMode,
}

impl PdfDocumentLoader {
    /// A loader on the born-digital text path (no page rendering).
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A loader with an explicit [`RasterMode`] (feature `render`).
    ///
    /// [`RasterMode`]: crate::primitive::RasterMode
    #[cfg(feature = "render")]
    pub(crate) fn with_raster(raster: RasterMode) -> Self {
        Self { raster }
    }

    /// Build a [`Document`] from the decoded `state`, `document` bytes, and
    /// `blobs` (the surfaced embedded-image / scanned-page parts).
    pub(super) fn document(document: Bytes, state: PdfState, blobs: Vec<DocumentPart>) -> Document {
        let mut parts = Vec::with_capacity(blobs.len() + 1);
        let stream: Box<dyn Stream<Text>> = Box::new(PdfStream {
            state: state.clone(),
            cursor: 0,
        });
        parts.push(DocumentPart::Stream {
            id: LocalId::new(BODY_PART_ID),
            handle: ErasedStream::new(FORMAT_ID.clone(), stream),
        });
        parts.extend(blobs);
        Document::new(
            FORMAT_ID.clone(),
            parts,
            Box::new(PdfRecombine { document, state }),
        )
    }
}

#[async_trait::async_trait]
impl DocumentLoader for PdfDocumentLoader {
    async fn decode(&self, content: ContentData) -> Result<Document> {
        let document = content.to_bytes();

        // `RasterMode::Always` (feature `render`): observe each page, its text
        // comes from the renderer alongside its glyph geometry, so redaction on
        // encode fills the detected pixels and emits a fresh image-only PDF (the
        // flatten guarantee, no selectable text). No blobs are surfaced: the whole
        // page is flattened, so per-image redaction is redundant.
        #[cfg(feature = "render")]
        if self.raster.render_dpi().is_some() {
            let observations = observe_pages(&document)?;
            let pages = pages_from_blocks(
                observations
                    .iter()
                    .map(|o| Block::new(o.page, o.text.clone())),
            );
            let state = PdfState::new(PdfInner {
                pages,
                deletions: Vec::new(),
                mode: RedactMode::Raster {
                    observations,
                    detections: Vec::new(),
                },
            });
            return Ok(Self::document(document, state, Vec::new()));
        }

        // Default (`Auto`/`Never`, and the whole pure-Rust build): glyph
        // deletion. The page text comes from `extract`, the same content walk
        // `redact_text` uses, so a detection's character span maps to the glyphs
        // it drew. On encode the glyphs are deleted and annotations/metadata
        // stripped, keeping a selectable text layer.
        let pdf = Pdf::open(&document)?;
        // Extract once: `blocks` drive glyph deletion, `embeddings` become image
        // blobs, and (in `Auto`) `issues` name the textless pages to render. A
        // second `extract()` would re-walk every page and re-copy the embedded
        // image bytes for nothing, so move each field out of this one result.
        let extraction = pdf.extract();
        let pages = pages_from_blocks(extraction.blocks);

        // Surface each redactable embedded image XObject as an image blob for the
        // image pipeline (only with an image codec able to redact them). Only
        // images whose bytes are a decodable file are surfaced (see
        // `embedding_hint`).
        // `mut` is used only when the scanned-page fold below (render + image) is
        // compiled in; under `image` alone nothing else extends the list.
        #[cfg(feature = "image")]
        #[cfg_attr(not(feature = "render"), allow(unused_mut))]
        let mut blobs: Vec<DocumentPart> = extraction
            .embeddings
            .into_iter()
            .filter_map(|embedding| {
                let hint = embedding_hint(embedding.kind)?;
                Some(DocumentPart::Blob {
                    id: LocalId::new(image_part_id(embedding.id)),
                    bytes: embedding.bytes,
                    hint: hint.to_string(),
                })
            })
            .collect();
        #[cfg(not(feature = "image"))]
        let blobs: Vec<DocumentPart> = Vec::new();

        // `RasterMode::Auto` (feature `render` + an image codec): a textless
        // (scanned) page has no glyphs to delete, so render it and surface it as
        // an image blob for the image pipeline to OCR and redact, while
        // born-digital pages keep glyph deletion. This is the Auto promise: text
        // where present, image where absent.
        #[cfg(feature = "render")]
        if matches!(self.raster, RasterMode::Auto) {
            for (number, png) in scanned_pages(&pdf, &extraction.issues)? {
                blobs.push(DocumentPart::Blob {
                    id: LocalId::new(page_part_id(number)),
                    bytes: png,
                    hint: "png".to_string(),
                });
            }
        }

        let state = PdfState::new(PdfInner {
            pages,
            deletions: Vec::new(),
            mode: RedactMode::GlyphDelete,
        });
        Ok(Self::document(document, state, blobs))
    }
}

/// Render each textless (`NeedsOcr`) page to a PNG, keyed by page number, to be
/// surfaced as an image blob.
#[cfg(feature = "render")]
fn scanned_pages(
    pdf: &Pdf,
    issues: &[crate::extract::Issue],
) -> Result<std::collections::BTreeMap<u32, Bytes>> {
    use crate::extract::IssueKind;

    // Which 1-based pages have no text layer.
    let textless: std::collections::BTreeSet<u32> = issues
        .iter()
        .filter(|issue| matches!(issue.kind, IssueKind::NeedsOcr))
        .map(|issue| issue.page)
        .collect();
    if textless.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }

    // Render only the textless pages, not the whole document: a mostly
    // born-digital PDF with a few scanned pages pays to rasterise just those.
    const RASTER_SCALE: f32 = 2.0;
    let rendered = pdf.render_pages(textless, RASTER_SCALE)?;
    Ok(rendered
        .into_iter()
        .map(|(number, page)| (number, Bytes::from(page.png)))
        .collect())
}

/// Assemble [`PdfPage`]s from the engine's per-page text [`Block`]s, assigning
/// each its start offset in the concatenated text stream.
///
/// Pages are separated by [`PAGE_SEPARATOR`] in the stream coordinate space: the
/// cumulative offset advances by each page's length *plus* the separator width,
/// so no detected span can straddle two pages (which encode would then drop).
fn pages_from_blocks(blocks: impl IntoIterator<Item = Block>) -> Vec<PdfPage> {
    let mut pages = Vec::new();
    let mut offset = 0usize;
    for block in blocks {
        let text = block.text.to_string();
        let len = text.len();
        pages.push(PdfPage {
            number: block.page,
            text,
            start: offset,
        });
        offset += len + PAGE_SEPARATOR.len();
    }
    pages
}

/// Observe every page for raster redaction: render it to pixels and extract its
/// text-layer glyph geometry, so the page text and glyph boxes share one
/// coordinate system.
#[cfg(feature = "render")]
fn observe_pages(document: &[u8]) -> Result<Vec<PageObservation>> {
    // A default render scale; higher scales trade output size for fidelity.
    const RASTER_SCALE: f32 = 2.0;
    Pdf::open(document).and_then(|pdf| pdf.observe(RASTER_SCALE))
}

/// The local part-id string for an image XObject: `"img-{number}-{generation}"`.
#[cfg(feature = "image")]
pub(super) fn image_part_id(id: ImageId) -> String {
    format!("img-{}-{}", id.number, id.generation)
}

/// Parse an image part-id string back into an [`ImageId`].
#[cfg(feature = "image")]
pub(super) fn parse_image_part_id(s: &str) -> Option<ImageId> {
    let rest = s.strip_prefix("img-")?;
    let (number, generation) = rest.split_once('-')?;
    Some(ImageId::new(number.parse().ok()?, generation.parse().ok()?))
}

/// The local part-id string for a scanned page: `"page-{number}"`. A distinct
/// namespace from embedded images (`img-{number}-{generation}`).
#[cfg(feature = "render")]
fn page_part_id(number: u32) -> String {
    format!("page-{number}")
}

/// Parse a scanned-page part-id string back into its 1-based page number.
#[cfg(feature = "render")]
pub(super) fn parse_page_part_id(s: &str) -> Option<u32> {
    s.strip_prefix("page-")?.parse().ok()
}

/// A filename-extension hint for an embedded image whose *raw stream bytes* are
/// a self-contained image file the orchestrator can decode, or `None` when they
/// are not.
///
/// An [`Embedding`](crate::extract::Embedding)'s bytes are the raw XObject
/// stream. For a JPEG (`DCTDecode`) or JPEG 2000 (`JPXDecode`) image those bytes
/// *are* a standalone `.jpg`/`.jp2` file that decodes directly. For the other
/// kinds, raw/`FlateDecode` samples, CCITT fax, JBIG2, the bytes are filter-
/// specific pixel data that only means anything alongside the XObject's
/// dictionary, so they are **not** a decodable file. Surfacing those with a
/// bogus extension would have the pipeline fail to decode and silently skip
/// them; returning `None` keeps them out of the container entirely, so their
/// non-redaction is an explicit (currently unsupported) case rather than a
/// silent miss.
#[cfg(feature = "image")]
fn embedding_hint(kind: EmbeddingKind) -> Option<&'static str> {
    match kind {
        EmbeddingKind::Jpeg => Some("jpg"),
        EmbeddingKind::Jpeg2000 => Some("jp2"),
        // Raw/Flate/CCITT/JBIG2: the stream bytes are not a self-contained
        // image file. Not surfaced (redacting these is not yet supported).
        EmbeddingKind::CcittFax | EmbeddingKind::Jbig2 | EmbeddingKind::Raw => None,
    }
}

#[cfg(all(test, feature = "image"))]
mod tests {
    use bytes::Bytes;
    use elide_codec::content::ContentData;
    use elide_codec::{DocumentLoader as _, DocumentPart, LocalId};
    use lopdf::content::{Content, Operation};
    use lopdf::{Dictionary, Document, Object, Stream, dictionary};

    use super::{PdfDocumentLoader, image_part_id, parse_image_part_id};
    use crate::document::Pdf;
    use crate::extract::ImageId;

    /// A JPEG-encoded image of a solid colour, as bytes (a self-contained
    /// `.jpg` file, so the loader surfaces it, see `embedding_hint`).
    fn jpeg(rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb(rgb));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }

    /// A one-page PDF with a single embedded JPEG (`DCTDecode`) image XObject,
    /// the kind whose raw stream bytes are a decodable file. Returns the
    /// document bytes and the image's `(number, generation)` id.
    fn image_pdf() -> (Vec<u8>, (u32, u16)) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let jpeg = jpeg([255, 255, 255]);
        let mut img = Dictionary::new();
        img.set("Type", Object::Name(b"XObject".to_vec()));
        img.set("Subtype", Object::Name(b"Image".to_vec()));
        img.set("Width", Object::Integer(8));
        img.set("Height", Object::Integer(8));
        img.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
        img.set("BitsPerComponent", Object::Integer(8));
        img.set("Filter", Object::Name(b"DCTDecode".to_vec()));
        let image_id = doc.add_object(Stream::new(img, jpeg));
        let res = doc.add_object(dictionary! { "XObject" => dictionary! { "Im1" => image_id } });
        let content = Content {
            operations: vec![Operation::new("Do", vec![Object::Name(b"Im1".to_vec())])],
        };
        let cid = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => cid, "Resources" => res,
        });
        let pages = dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", cat);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        (out, image_id)
    }

    fn black_png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([0, 0, 0]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// The image blob parts a document surfaces, as `(id, bytes)`.
    /// The embedded-image blobs (`img-*`) the document surfaces, excluding any
    /// scanned-page (`page-*`) blobs the render path also adds.
    fn image_blobs(doc: &elide_codec::Document) -> Vec<(String, Bytes)> {
        doc.parts()
            .iter()
            .filter_map(|part| match part {
                DocumentPart::Blob { id, bytes, .. } if id.as_str().starts_with("img-") => {
                    Some((id.as_str().to_owned(), bytes.clone()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn part_id_round_trips() {
        let id = ImageId::new(7, 3);
        assert_eq!(parse_image_part_id(&image_part_id(id)), Some(id));
        assert_eq!(parse_image_part_id("not-an-image"), None);
    }

    #[tokio::test]
    async fn document_surfaces_and_replaces_an_image() {
        let (pdf, image_id) = image_pdf();
        let mut doc = PdfDocumentLoader::new()
            .decode(ContentData::new(Bytes::from(pdf)))
            .await
            .unwrap();

        // The document surfaces the one embedded image as a blob part.
        let blobs = image_blobs(&doc);
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].0, format!("img-{}-{}", image_id.0, image_id.1));

        // Fold a redacted (black) image back into the document.
        doc.replace_part(&LocalId::new(blobs[0].0.clone()), Bytes::from(black_png()))
            .unwrap();

        // Encode applies it: the output's image bytes differ from the original.
        let original = jpeg([255, 255, 255]);
        let encoded = doc.encode().unwrap();
        let after = Pdf::open(encoded.as_bytes()).unwrap().extract();
        assert_eq!(after.embeddings.len(), 1);
        assert_ne!(
            after.embeddings[0].bytes.as_ref(),
            original.as_slice(),
            "image was not redacted"
        );
    }

    #[tokio::test]
    async fn replace_part_rejects_an_unknown_id() {
        let (pdf, _) = image_pdf();
        let mut doc = PdfDocumentLoader::new()
            .decode(ContentData::new(Bytes::from(pdf)))
            .await
            .unwrap();
        let err = doc
            .replace_part(&LocalId::new("img-9999-0"), Bytes::from(black_png()))
            .unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::MalformedInput);
    }
}

#[cfg(all(test, feature = "render"))]
mod raster_tests {
    use bytes::Bytes;
    use elide_codec::{DocumentPart, LocalId};

    use super::super::RedactMode;
    use super::super::pdf_state::{PdfInner, PdfState};
    use super::PdfDocumentLoader;

    /// A raster document flattens every page, so it surfaces no blob parts and
    /// `replace_part` rejects any id (the raster path never accepts a per-image
    /// replacement). Built directly with empty observations, so it needs no
    /// PDFium (the observe step runs only in the loader's decode).
    #[test]
    fn raster_surfaces_no_blob_parts_and_rejects_replacement() {
        let state = PdfState::new(PdfInner {
            pages: Vec::new(),
            deletions: Vec::new(),
            mode: RedactMode::Raster {
                observations: Vec::new(),
                detections: Vec::new(),
            },
        });
        let mut doc = PdfDocumentLoader::document(Bytes::new(), state, Vec::new());

        // No blob parts: only the body stream.
        assert!(
            doc.parts()
                .iter()
                .all(|p| matches!(p, DocumentPart::Stream { .. })),
            "raster mode surfaced a blob part"
        );

        // `replace_part` rejects any id (no blob to replace).
        let err = doc
            .replace_part(&LocalId::new("img-1-0"), Bytes::from_static(b"x"))
            .unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::MalformedInput);
    }
}

/// The scanned-page image-part path (`RasterMode::Auto`), exercised by decoding a
/// scanned (textless) one-page PDF: the document surfaces the page as an image
/// blob, accepts a redacted replacement, and reflattens it on encode.
/// The scanned-page image-part path (`RasterMode::Auto`): the document surfaces a
/// textless page as an image blob, accepts a redacted replacement, and reflattens
/// it on encode. Built directly with a pre-rendered raster (as the loader's render
/// step would produce), so it needs no PDFium.
#[cfg(all(test, feature = "render", feature = "image"))]
mod scanned_page_tests {
    use std::io::Cursor;

    use bytes::Bytes;
    use elide_codec::{DocumentPart, LocalId};

    use super::super::RedactMode;
    use super::super::pdf_state::{PdfInner, PdfState};
    use super::PdfDocumentLoader;

    /// A solid-colour PNG of `w`x`h`.
    fn png(w: u32, h: u32, colour: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb(colour));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A one-page image-only (textless) PDF whose content stream carries a marker
    /// so we can tell whether the original page content survives a reflatten.
    fn scanned_doc() -> Bytes {
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.5");
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

    /// Build the `Auto`-path document directly, standing in for the loader's
    /// render step by handing the scanned page's raster as a `page-N` blob, so no
    /// PDFium is needed.
    fn scanned_document(document: Bytes, scanned: &[(u32, Vec<u8>)]) -> elide_codec::Document {
        let state = PdfState::new(PdfInner {
            pages: Vec::new(),
            deletions: Vec::new(),
            mode: RedactMode::GlyphDelete,
        });
        let blobs = scanned
            .iter()
            .map(|(number, png)| DocumentPart::Blob {
                id: LocalId::new(format!("page-{number}")),
                bytes: Bytes::from(png.clone()),
                hint: "png".to_owned(),
            })
            .collect();
        PdfDocumentLoader::document(document, state, blobs)
    }

    #[test]
    fn surfaces_a_scanned_page_and_reflattens_it_on_encode() {
        let mut doc = scanned_document(scanned_doc(), &[(1, png(100, 100, [200, 200, 200]))]);

        // The document surfaces the scanned page as a `page-1` image blob.
        let page_blob = doc.parts().iter().find_map(|p| match p {
            DocumentPart::Blob { id, hint, .. } if id.as_str() == "page-1" => Some(hint.clone()),
            _ => None,
        });
        assert_eq!(
            page_blob.as_deref(),
            Some("png"),
            "scanned page not surfaced"
        );

        // Feed back a redacted (black) raster for that page.
        let redacted = Bytes::from(png(100, 100, [0, 0, 0]));
        doc.replace_part(&LocalId::new("page-1"), redacted)
            .expect("page-1 accepted");

        // Encode reflattens the page: the original page content is gone.
        let out = doc.encode().unwrap();
        let bytes = out.to_bytes();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("SCANNED-PAGE-MARKER"),
            "original scanned page content survived reflatten"
        );
        // The output still opens as a one-page PDF (the reflattened page is
        // image-only, so it carries no text blocks).
        let reopened = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(reopened.get_pages().len(), 1);
    }

    #[test]
    fn rejects_a_page_id_the_document_did_not_surface() {
        // No scanned pages were surfaced, so `page-1` is not accepted.
        let mut doc = scanned_document(scanned_doc(), &[]);
        let err = doc
            .replace_part(&LocalId::new("page-1"), Bytes::from_static(b"x"))
            .unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::MalformedInput);
    }
}
