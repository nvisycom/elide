//! The PDF [`DocumentLoader`]: decode page text and, on the glyph path,
//! surface the embedded images, attachments, and scanned pages as blobs.

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{
    Document, DocumentLoader, DocumentPart, ErasedStream, LocalId, Stream, StrippedContent,
};
use elide_core::Result;
use elide_core::modality::text::Text;
use elide_core::redaction::HiddenContentPolicy;

use super::part_id::PartId;
use super::recombine::PdfRecombine;
use super::state::{PdfInner, PdfState};
use super::stream::PdfStream;
use super::{BODY_PART_ID, FORMAT_ID, PdfPage, RedactMode};
use crate::document::Pdf;
use crate::document::extract::Block;
#[cfg(feature = "render")]
use crate::primitive::RasterMode;
#[cfg(feature = "render")]
use crate::render::PageObservation;
use crate::text::PageGeometry;

/// A [`DocumentLoader`] that decodes a PDF into a [`Document`]: the page-text
/// body [`Stream`] plus, on the glyph-deletion path, a
/// [`Blob`](DocumentPart::Blob) per redactable embedded image and (feature
/// `render`) per textless scanned page.
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

    /// Build a [`Document`] from the decoded `state`, the (sanitized)
    /// `document` bytes, and `blobs` (the surfaced embedded-image / scanned-page
    /// parts).
    pub(super) fn document(document: Bytes, state: PdfState, blobs: Vec<DocumentPart>) -> Document {
        let mut parts = Vec::with_capacity(blobs.len() + 1);
        let stream: Box<dyn Stream<Text>> = Box::new(PdfStream {
            state: state.clone(),
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
    async fn decode(&self, content: ContentData, _policy: HiddenContentPolicy) -> Result<Document> {
        // Hidden content goes first, so everything below (extraction, rendering,
        // the surfaced images) reads only what will ship.
        let opened = Pdf::open(content.as_bytes())?;
        let (document, pdf, stripped) = match opened.sanitize()? {
            Some((bytes, stripped)) => {
                let pdf = Pdf::open(&bytes)?;
                (Bytes::from(bytes), pdf, stripped)
            }
            None => (content.to_bytes(), opened, StrippedContent::new()),
        };

        // `RasterMode::Always` (feature `render`): observe each page, its text
        // comes from the renderer alongside its glyph geometry, so redaction on
        // encode fills the detected pixels and emits a fresh image-only PDF (the
        // flatten guarantee, no selectable text). No blobs are surfaced: the whole
        // page is flattened, so per-image redaction is redundant.
        #[cfg(feature = "render")]
        if self.raster.render_dpi().is_some() {
            let observations = observe_pages(&document)?;
            // The raster path flattens every page to an image, so per-entity
            // highlight geometry on the text location is not surfaced here.
            let pages = pages_from_blocks(
                observations
                    .iter()
                    .map(|o| Block::new(o.page, o.text.clone())),
                Vec::new(),
            );
            let state = PdfState::new(PdfInner {
                pages,
                deletions: Vec::new(),
                mode: RedactMode::Raster {
                    observations,
                    detections: Vec::new(),
                },
            });
            return Ok(Self::document(document, state, Vec::new()).with_stripped(stripped));
        }

        // Default (`Auto`/`Never`, and the whole pure-Rust build): glyph
        // deletion. The page text comes from `extract`, the same content walk
        // `redact_text` uses, so a detection's character span maps to the glyphs
        // it drew. On encode the glyphs are deleted, keeping a selectable text
        // layer.
        //
        // Extract once: `blocks` drive glyph deletion, `embeddings` become image
        // blobs, and (in `Auto`) `issues` name the textless pages to render. A
        // second `extract()` would re-walk every page and re-copy the embedded
        // image bytes for nothing, so move each field out of this one result.
        let (extraction, geometry) = pdf.extract();
        let pages = pages_from_blocks(extraction.blocks, geometry);

        // Surface each embedded image XObject as an image blob for the image
        // pipeline. Only images the pipeline can read remain (the sanitizer
        // removed the rest, see `image::surface`); one no codec can read is
        // removed on encode.
        let mut blobs = Vec::new();
        for embedding in extraction.embeddings {
            let Some((bytes, hint)) =
                pdf.surface_image(embedding.id, embedding.kind, embedding.bytes)?
            else {
                continue;
            };
            blobs.push(DocumentPart::Blob {
                id: LocalId::new(PartId::Image(embedding.id).to_string()),
                bytes,
                hint: hint.to_owned(),
            });
        }

        // Each attached file is content of its own format, decoded and redacted
        // through the registry like any embedding.
        blobs.extend(
            pdf.attachments()?
                .into_iter()
                .map(|attachment| DocumentPart::Blob {
                    id: LocalId::new(PartId::Attachment(attachment.id).to_string()),
                    hint: attachment_hint(&attachment.name),
                    bytes: attachment.bytes,
                }),
        );

        // `RasterMode::Auto` (feature `render` + an image codec): a textless
        // (scanned) page has no glyphs to delete, so render it and surface it as
        // an image blob for the image pipeline to OCR and redact, while
        // born-digital pages keep glyph deletion. This is the Auto promise: text
        // where present, image where absent.
        #[cfg(feature = "render")]
        if matches!(self.raster, RasterMode::Auto) {
            for (number, png) in scanned_pages(&pdf, &extraction.issues)? {
                blobs.push(DocumentPart::Blob {
                    id: LocalId::new(PartId::Page(number).to_string()),
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
        Ok(Self::document(document, state, blobs).with_stripped(stripped))
    }
}

/// Render each textless (`NeedsOcr`) page to a PNG, keyed by page number, to be
/// surfaced as an image blob.
#[cfg(feature = "render")]
fn scanned_pages(
    pdf: &Pdf,
    issues: &[crate::document::extract::Issue],
) -> Result<std::collections::BTreeMap<u32, Bytes>> {
    use crate::document::extract::IssueKind;

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

/// Assemble [`PdfPage`]s from the engine's per-page text [`Block`]s, attaching
/// each page's rendered glyph geometry (matched by page number).
///
/// Each page is addressed in its own page-local byte space (a located entity
/// carries the page number), so there is no cross-page offset and a detected
/// span cannot straddle two pages.
fn pages_from_blocks(
    blocks: impl IntoIterator<Item = Block>,
    geometry: Vec<PageGeometry>,
) -> Vec<PdfPage> {
    let mut by_page: BTreeMap<u32, Arc<PageGeometry>> = geometry
        .into_iter()
        .map(|g| (g.page, Arc::new(g)))
        .collect();
    let mut pages = Vec::new();
    for block in blocks {
        pages.push(PdfPage {
            number: block.page,
            text: block.text.to_string(),
            geometry: by_page.remove(&block.page),
        });
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

/// The registry hint for an attachment named `name`: its lowercased file
/// extension, empty when it has none (no codec then reads it, so it is removed).
fn attachment_hint(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The document a decode assembles, built directly from a hand-made state with
/// a pre-rendered raster (as the render step would produce), so these need no
/// PDFium.
#[cfg(all(test, feature = "render"))]
mod tests {
    /// Raster mode through the codec: every page flattened to a redacted image.
    mod raster {
        use bytes::Bytes;
        use elide_codec::{DocumentPart, LocalId};

        use super::super::PdfDocumentLoader;
        use crate::codec::RedactMode;
        use crate::codec::state::{PdfInner, PdfState};

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

    /// The scanned-page image-part path (`RasterMode::Auto`): the document surfaces a
    /// textless page as an image blob, accepts a redacted replacement, and reflattens
    /// it on encode. Built directly with a pre-rendered raster (as the loader's render
    /// step would produce), so it needs no PDFium.
    mod scanned {
        use std::io::Cursor;

        use bytes::Bytes;
        use elide_codec::{DocumentPart, LocalId};

        use super::super::PdfDocumentLoader;
        use crate::codec::RedactMode;
        use crate::codec::state::{PdfInner, PdfState};

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
                DocumentPart::Blob { id, hint, .. } if id.as_str() == "page-1" => {
                    Some(hint.clone())
                }
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
}
