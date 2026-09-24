//! PDF handler: adapts the [`Pdf`] engine to the codec's [`Handler`]
//! contract.
//!
//! Each page's text is streamed as one [`Chunk`]; a redaction on that chunk is
//! recorded and applied on [`encode`] per the handler's [`RedactMode`]:
//!
//! - **Glyph deletion** (default, pure-Rust): the detected glyphs are deleted
//!   from the content streams and annotations/metadata stripped, keeping a
//!   selectable text layer with the detected spans gone
//!   ([`Pdf::redact_text`]).
//! - **Raster** (feature `render`, [`RasterMode::Always`]): the page text
//!   comes from [`Pdf::observe`] alongside its glyph
//!   geometry, so a redaction's span maps to pixel boxes; encode fills them and
//!   emits a fresh image-only PDF, the text layer is gone entirely.
//!
//! [`encode`]: Handler::encode
//! [`RasterMode::Always`]: super::RasterMode::Always

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{Container, Format, FormatId, Handler, LocalId, Part};
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};

use super::PdfLoader;
#[cfg(feature = "render")]
use super::RasterMode;
use crate::Pdf;
#[cfg(feature = "image")]
use crate::extract::{EmbeddingKind, ImageId};
use crate::redact::Detection;
#[cfg(feature = "image")]
use crate::redact::ImageReplacement;
#[cfg(all(feature = "render", feature = "image"))]
use crate::redact::PageReplacement;
#[cfg(feature = "render")]
use crate::render::PageObservation;

/// Stable [`FormatId`] for the PDF codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pdf");

/// [`Format`] descriptor registered into `FormatRegistry`.
///
/// Decodes on the glyph-deletion redaction path. To flatten pages to images
/// instead, build the format with `format_with` and `RasterMode::Always`.
pub fn format() -> Format {
    Format::new(FORMAT_ID.clone(), PdfLoader::new())
        .with_extensions(["pdf"])
        .with_content_types(["application/pdf"])
}

/// [`Format`] descriptor with an explicit [`RasterMode`].
///
/// Under [`RasterMode::Always`] redaction flattens every page to an image
/// (a fresh image-only PDF); [`Auto`](RasterMode::Auto) and
/// [`Never`](RasterMode::Never) use the default glyph-deletion path.
///
/// [`RasterMode::Always`]: super::RasterMode::Always
#[cfg(feature = "render")]
#[cfg_attr(docsrs, doc(cfg(feature = "render")))]
pub fn format_with(raster: RasterMode) -> Format {
    Format::new(FORMAT_ID.clone(), PdfLoader::with_raster(raster))
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

/// How a [`PdfHandler`] applies recorded redactions on encode.
#[derive(Debug, Default)]
pub(crate) enum RedactMode {
    /// Glyph deletion via [`Pdf::redact_text`]: the
    /// detected glyphs are removed and annotations/metadata stripped, keeping a
    /// selectable text layer. The default pure-Rust redaction path.
    #[default]
    GlyphDelete,
    /// Raster redaction (feature `render`): the page text comes from
    /// [`Pdf::observe`], so a redaction's span maps
    /// directly to glyph pixel boxes; encode fills them and emits a fresh
    /// image-only PDF.
    #[cfg(feature = "render")]
    Raster {
        /// Per-page observations (text, glyph boxes, pixels) from `observe`.
        observations: Vec<PageObservation>,
        /// Recorded detections (page + character span), applied at encode.
        detections: Vec<Detection>,
    },
}

impl RedactMode {
    /// Whether redaction flattens pages to images (raster) rather than deleting
    /// glyphs in place.
    fn is_raster(&self) -> bool {
        !matches!(self, RedactMode::GlyphDelete)
    }
}

/// PDF text handler backed by [`Pdf`].
///
/// Streams each page's text as a chunk, records per-page redactions, and on
/// [`encode`](Handler::encode) applies them per its [`RedactMode`].
#[derive(Debug, Default)]
pub(crate) struct PdfHandler {
    /// The original document bytes, retained so [`Pdf`] re-serialises from
    /// the true source.
    pub(crate) document: Bytes,
    /// Extracted pages, in page order, with stream offsets for `read_next`.
    pub(crate) pages: Vec<PdfPage>,
    /// Read cursor over `pages`.
    pub(crate) cursor: usize,
    /// Recorded glyph-deletion detections (per-page character spans), applied at
    /// encode in [`RedactMode::GlyphDelete`].
    pub(crate) deletions: Vec<Detection>,
    /// The ids of the embedded images the [`Container`] surfaces as redactable
    /// (a decodable file), cached at decode so [`replace_part`](Container::replace_part)
    /// validates without re-extracting the document.
    ///
    /// [`replace_part`]: Container::replace_part
    #[cfg(feature = "image")]
    pub(crate) redactable_image_ids: std::collections::BTreeSet<ImageId>,
    /// Redacted replacement images, keyed by their XObject id, filled through
    /// the [`Container`] surface and applied on encode. Only meaningful with an
    /// image codec (`image`) able to redact the surfaced images.
    #[cfg(feature = "image")]
    pub(crate) image_replacements: std::collections::HashMap<ImageId, Bytes>,
    /// Textless (scanned) pages rendered to a PNG, keyed by 1-based page number,
    /// surfaced by the [`Container`] as image parts so the image pipeline OCRs
    /// and redacts them. Populated under [`RasterMode::Auto`] with `render`.
    #[cfg(all(feature = "render", feature = "image"))]
    pub(crate) scanned_pages: std::collections::BTreeMap<u32, Bytes>,
    /// Redacted replacement images for scanned pages, keyed by page number,
    /// filled through the [`Container`] surface and reflattened on encode.
    #[cfg(all(feature = "render", feature = "image"))]
    pub(crate) page_replacements: std::collections::HashMap<u32, Bytes>,
    /// How recorded redactions are applied on encode.
    pub(crate) mode: RedactMode,
}

impl PdfHandler {
    /// A glyph-deletion handler over the extracted `pages`: redaction deletes
    /// the detected glyphs, keeping a selectable text layer, and strips
    /// annotations and metadata. The default (pure-Rust) redaction path.
    pub(crate) fn text(document: Bytes, pages: Vec<PdfPage>) -> Self {
        Self {
            #[cfg(feature = "image")]
            redactable_image_ids: redactable_image_ids(&document),
            document,
            pages,
            mode: RedactMode::GlyphDelete,
            ..Self::default()
        }
    }

    /// A glyph-deletion handler that also surfaces `scanned_pages` (textless
    /// pages rendered to PNG) as image parts, so the image pipeline OCRs and
    /// redacts them while born-digital pages keep their selectable text. The
    /// [`RasterMode::Auto`] path.
    ///
    /// [`RasterMode::Auto`]: super::RasterMode::Auto
    #[cfg(all(feature = "render", feature = "image"))]
    pub(crate) fn text_auto(
        document: Bytes,
        pages: Vec<PdfPage>,
        scanned_pages: std::collections::BTreeMap<u32, Bytes>,
    ) -> Self {
        Self {
            redactable_image_ids: redactable_image_ids(&document),
            document,
            pages,
            scanned_pages,
            mode: RedactMode::GlyphDelete,
            ..Self::default()
        }
    }

    /// A raster-redaction handler: `pages` carry the observation text (offsets
    /// into `observations`' glyphs), redacted by pixel fill on encode.
    #[cfg(feature = "render")]
    pub(crate) fn raster(
        document: Bytes,
        pages: Vec<PdfPage>,
        observations: Vec<PageObservation>,
    ) -> Self {
        Self {
            document,
            pages,
            mode: RedactMode::Raster {
                observations,
                detections: Vec::new(),
            },
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
        match &self.mode {
            RedactMode::GlyphDelete => {
                #[cfg(feature = "image")]
                let has_images = !self.image_replacements.is_empty();
                #[cfg(not(feature = "image"))]
                let has_images = false;
                #[cfg(all(feature = "render", feature = "image"))]
                let has_pages = !self.page_replacements.is_empty();
                #[cfg(not(all(feature = "render", feature = "image")))]
                let has_pages = false;
                if self.deletions.is_empty() && !has_images && !has_pages {
                    return Ok(ContentData::new(self.document.clone()));
                }

                // Delete the detected glyphs and strip annotations/metadata,
                // keeping a selectable text layer. (`mut` is used only when the
                // image fold below is compiled in.)
                #[cfg_attr(not(feature = "image"), allow(unused_mut))]
                let mut out =
                    Pdf::open(&self.document).and_then(|pdf| pdf.redact_text(&self.deletions))?;

                // Then fold in any redacted embedded images.
                #[cfg(feature = "image")]
                if has_images {
                    let replacements: Vec<ImageReplacement> = self
                        .image_replacements
                        .iter()
                        .map(|(&id, bytes)| ImageReplacement {
                            id,
                            image: bytes.to_vec(),
                        })
                        .collect();
                    out = Pdf::open(&out).and_then(|pdf| pdf.redact_images(&replacements))?;
                }

                // Then reflatten any scanned pages to their redacted raster.
                #[cfg(all(feature = "render", feature = "image"))]
                if has_pages {
                    let replacements: Vec<PageReplacement> = self
                        .page_replacements
                        .iter()
                        .map(|(&number, bytes)| PageReplacement {
                            number,
                            image: bytes.to_vec(),
                        })
                        .collect();
                    out = Pdf::open(&out).and_then(|pdf| pdf.redact_pages(&replacements))?;
                }

                Ok(ContentData::new(Bytes::from(out)))
            }
            #[cfg(feature = "render")]
            RedactMode::Raster {
                observations,
                detections,
            } => {
                if detections.is_empty() {
                    return Ok(ContentData::new(self.document.clone()));
                }
                // Fill the detected glyph boxes and emit a fresh image-only PDF
                //, the strong redaction guarantee. Black fill.
                let (out, _certificate) = Pdf::open(&self.document)
                    .and_then(|pdf| pdf.redact_raster(observations, detections, [0, 0, 0]))?;
                Ok(ContentData::new(Bytes::from(out)))
            }
        }
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        if self.cursor >= self.pages.len() {
            return Ok(None);
        }
        let page = &self.pages[self.cursor];
        let chunk = Chunk {
            location: TextLocation::new(page.start, page.start + page.text.len())
                .with_page(Some(page.number)),
            data: TextData::new(page.text.clone()),
            hints: Vec::new(),
        };
        self.cursor += 1;
        Ok(Some(chunk))
    }

    fn lift(&self, chunk: &Chunk<Text>, local: TextLocation) -> Option<TextLocation> {
        let chunk_range = chunk.location.range()?;
        let local_range = local.range()?;
        let base = chunk_range.start;
        let start = base + local_range.start;
        let end = base + local_range.end;
        if start > end || end > chunk_range.end {
            return None;
        }
        // PDF text has no flat source byte coordinate (glyph runs in content
        // streams), so no source range is carried.
        Some(TextLocation::new(start, end).with_page(chunk.location.page))
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        Some(self)
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for PdfHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(range) = location.range() else {
            return Ok(None); // source-only location has no decoded range to read
        };
        let Some((page, local)) = self.page_at(range.start) else {
            return Ok(None);
        };
        let Some(local_end) = range.end.checked_sub(page.start) else {
            return Ok(None);
        };
        Ok(page.text.get(local..local_end).map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for PdfHandler {
    async fn write_at(&mut self, redactions: Redactions<Text>) -> Result<()> {
        for (location, _replacement) in redactions.into_iter() {
            // Resolve the redaction to the index of the page it falls on and its
            // byte range within that page's text. Both modes address glyphs by
            // the same character span into the page text (not replacement text),
            // so the span is measured once as character offsets below. PDF text
            // is addressed by decoded range only; a source-only location has
            // none, so it is skipped.
            let Some(range) = location.range() else {
                continue;
            };
            let Some((page_idx, local, local_end)) =
                self.pages.iter().enumerate().find_map(|(idx, page)| {
                    if range.start < page.start || range.start >= page.start + page.text.len() {
                        return None;
                    }
                    let local = range.start - page.start;
                    let local_end = range.end.checked_sub(page.start)?;
                    Some((idx, local, local_end))
                })
            else {
                continue;
            };
            let page = &self.pages[page_idx];
            if page.text.get(local..local_end).is_none() {
                continue; // range not on a char boundary
            }
            let page_number = page.number;

            // Both paths address glyphs by the same character span into the page
            // text, so the span is measured once as character offsets; drop the
            // page borrow before recording so the sink can be borrowed mutably.
            let start = page.text[..local].chars().count();
            let end = page.text[..local_end].chars().count();
            let detection = Detection::new(page_number, start, end);
            match &mut self.mode {
                #[cfg(feature = "render")]
                RedactMode::Raster { detections, .. } => detections.push(detection),
                _ => self.deletions.push(detection),
            }
        }
        Ok(())
    }
}

impl Container for PdfHandler {
    /// Surface each redactable embedded image XObject as a [`Part`] for the
    /// image pipeline.
    ///
    /// Only with an image codec (`image`) able to decode and redact the
    /// images, and only on the glyph-deletion path, the raster path flattens
    /// every page to an image, so surfacing images for separate redaction would
    /// be redundant. Only images whose bytes are a decodable file are surfaced
    /// (see [`embedding_hint`]).
    fn parts(&self) -> Vec<Part> {
        #[cfg(feature = "image")]
        {
            // In raster mode the whole page is flattened, so per-image redaction
            // is redundant, surface nothing.
            #[cfg(feature = "render")]
            if matches!(self.mode, RedactMode::Raster { .. }) {
                return Vec::new();
            }
            let Ok(pdf) = Pdf::open(&self.document) else {
                return Vec::new();
            };
            let mut parts: Vec<Part> = pdf
                .extract()
                .embeddings
                .into_iter()
                .filter_map(|embedding| {
                    let hint = embedding_hint(embedding.kind)?;
                    Some(Part {
                        id: LocalId::new(image_part_id(embedding.id)),
                        bytes: embedding.bytes,
                        hint: hint.to_string(),
                    })
                })
                .collect();

            // Textless (scanned) pages, rendered to a PNG, are surfaced as image
            // parts so the image pipeline OCRs and redacts them. Their part ids
            // (`page-N`) are a distinct namespace from embedded images (`img-N-G`).
            #[cfg(feature = "render")]
            for (&number, png) in &self.scanned_pages {
                parts.push(Part {
                    id: LocalId::new(page_part_id(number)),
                    bytes: png.clone(),
                    hint: "png".to_string(),
                });
            }

            parts
        }
        #[cfg(not(feature = "image"))]
        Vec::new()
    }

    fn replace_part(&mut self, id: &LocalId, bytes: Bytes) -> Result<()> {
        // In raster mode the output is a flattened image-only PDF, so no
        // per-image replacement is surfaced or applied, reject fail-closed
        // before accepting any bytes.
        if self.mode.is_raster() {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("pdf replace_part: `{id}` is not accepted in raster mode"),
            ));
        }
        #[cfg(feature = "image")]
        {
            // A scanned page's redacted raster (`page-N`): store it for the
            // encode-time reflatten. Validated against the surfaced set.
            #[cfg(feature = "render")]
            if let Some(number) =
                parse_page_part_id(id.as_str()).filter(|n| self.scanned_pages.contains_key(n))
            {
                self.page_replacements.insert(number, bytes);
                return Ok(());
            }

            // Accept only ids naming an image the container surfaced, validated
            // against the cached id set (no re-extraction per call).
            let image_id = parse_image_part_id(id.as_str())
                .filter(|id| self.redactable_image_ids.contains(id));
            if let Some(image_id) = image_id {
                self.image_replacements.insert(image_id, bytes);
                return Ok(());
            }
        }
        #[cfg(not(feature = "image"))]
        let _ = bytes;
        Err(Error::new(
            ErrorKind::MalformedInput,
            format!("pdf replace_part: `{id}` is not a redactable embedded image"),
        ))
    }
}

/// The ids of every embedded image whose bytes are a decodable file (see
/// [`embedding_hint`]), the set the container will surface and accept back.
#[cfg(feature = "image")]
fn redactable_image_ids(document: &[u8]) -> std::collections::BTreeSet<ImageId> {
    let Ok(pdf) = Pdf::open(document) else {
        return std::collections::BTreeSet::new();
    };
    pdf.extract()
        .embeddings
        .iter()
        .filter(|e| embedding_hint(e.kind).is_some())
        .map(|e| e.id)
        .collect()
}

/// The local part-id string for an image XObject: `"img-{number}-{generation}"`.
#[cfg(feature = "image")]
fn image_part_id(id: ImageId) -> String {
    format!("img-{}-{}", id.number, id.generation)
}

/// Parse an image part-id string back into an [`ImageId`].
#[cfg(feature = "image")]
fn parse_image_part_id(s: &str) -> Option<ImageId> {
    let rest = s.strip_prefix("img-")?;
    let (number, generation) = rest.split_once('-')?;
    Some(ImageId::new(number.parse().ok()?, generation.parse().ok()?))
}

/// The local part-id string for a scanned page: `"page-{number}"`. A distinct
/// namespace from embedded images (`img-{number}-{generation}`).
#[cfg(all(feature = "render", feature = "image"))]
fn page_part_id(number: u32) -> String {
    format!("page-{number}")
}

/// Parse a scanned-page part-id string back into its 1-based page number.
#[cfg(all(feature = "render", feature = "image"))]
fn parse_page_part_id(s: &str) -> Option<u32> {
    s.strip_prefix("page-")?.parse().ok()
}

/// A filename-extension hint for an embedded image whose *raw stream bytes* are
/// a self-contained image file the orchestrator can decode, or `None` when they
/// are not.
///
/// An [`Embedding`]'s bytes are the raw XObject
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
    use elide_codec::{Container, Handler, LocalId};
    use elide_core::ErrorKind;
    use lopdf::content::{Content, Operation};
    use lopdf::{Dictionary, Document, Object, Stream, dictionary};

    use super::{FORMAT_ID, PdfHandler, PdfPage, image_part_id, parse_image_part_id};
    use crate::Pdf;
    use crate::extract::ImageId;

    /// A JPEG-encoded image of a solid colour, as bytes (a self-contained
    /// `.jpg` file, so the container surfaces it, see `embedding_hint`).
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

    #[test]
    fn part_id_round_trips() {
        let id = ImageId::new(7, 3);
        assert_eq!(parse_image_part_id(&image_part_id(id)), Some(id));
        assert_eq!(parse_image_part_id("not-an-image"), None);
    }

    #[test]
    fn container_surfaces_and_replaces_an_image() {
        let (pdf, image_id) = image_pdf();
        let mut handler = PdfHandler::text(Bytes::from(pdf), Vec::<PdfPage>::new());

        // parts() surfaces the one embedded image.
        let parts = handler.parts();
        assert_eq!(parts.len(), 1);
        assert_eq!(
            parts[0].id.as_str(),
            format!("img-{}-{}", image_id.0, image_id.1)
        );

        // Fold a redacted (black) image back through the container.
        let part_id = parts[0].id.clone();
        handler
            .replace_part(&part_id, Bytes::from(black_png()))
            .unwrap();

        // Encode applies it: the output's image bytes differ from the original.
        let original = jpeg([255, 255, 255]);
        let encoded = handler.encode().unwrap();
        let after = Pdf::open(encoded.as_bytes()).unwrap().extract();
        assert_eq!(after.embeddings.len(), 1);
        assert_ne!(
            after.embeddings[0].bytes.as_ref(),
            original.as_slice(),
            "image was not redacted"
        );
        assert_eq!(handler.format(), FORMAT_ID.clone());
    }

    #[test]
    fn replace_part_rejects_an_unknown_id() {
        let (pdf, _) = image_pdf();
        let mut handler = PdfHandler::text(Bytes::from(pdf), Vec::<PdfPage>::new());
        let err = handler
            .replace_part(&LocalId::new("img-9999-0"), Bytes::from(black_png()))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }
}

#[cfg(all(test, feature = "render"))]
mod raster_tests {
    use bytes::Bytes;
    use elide_codec::{Container, LocalId};
    use elide_core::ErrorKind;

    use super::{PdfHandler, PdfPage};

    /// A raster handler built from empty observations (no PDFium needed):
    /// `parts()` surfaces nothing and `replace_part` rejects any id, since the
    /// raster path flattens every page and never accepts a per-image replacement.
    #[test]
    fn raster_surfaces_no_parts_and_rejects_replacement() {
        let mut handler = PdfHandler::raster(Bytes::new(), Vec::<PdfPage>::new(), Vec::new());

        assert!(handler.parts().is_empty());

        let err = handler
            .replace_part(&LocalId::new("img-1-0"), Bytes::from_static(b"x"))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }
}

/// The scanned-page image-part path (`RasterMode::Auto`), exercised without
/// PDFium by handing the handler a pre-rendered page raster directly. Proves the
/// container surfaces a textless page as an image part, accepts a redacted
/// replacement, and reflattens it on encode, with no OCR backend involved.
#[cfg(all(test, feature = "render", feature = "image"))]
mod scanned_page_tests {
    use std::collections::BTreeMap;
    use std::io::Cursor;

    use bytes::Bytes;
    use elide_codec::content::ContentData;
    use elide_codec::{Container, Handler, LocalId};
    use lopdf::{Document, Object, Stream, dictionary};

    use super::{PdfHandler, PdfPage};
    use crate::Pdf;

    /// A solid-colour PNG of `w`x`h`.
    fn png(w: u32, h: u32, colour: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb(colour));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    /// A one-page image-only PDF whose content stream carries a marker so we can
    /// tell whether the original page content survives a reflatten.
    fn scanned_doc() -> Bytes {
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

    #[tokio::test]
    async fn surfaces_a_scanned_page_and_reflattens_it_on_encode() {
        let document = scanned_doc();
        // Stand in for the loader's render step: page 1 is textless, so it is
        // supplied as a rendered raster.
        let mut scanned = BTreeMap::new();
        scanned.insert(1u32, Bytes::from(png(100, 100, [200, 200, 200])));
        let mut handler = PdfHandler::text_auto(document, Vec::<PdfPage>::new(), scanned);

        // The container surfaces the scanned page as a `page-1` image part.
        let parts = handler.parts();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].id.as_str(), "page-1");
        assert_eq!(parts[0].hint, "png");

        // Feed back a redacted (black) raster for that page.
        let redacted = Bytes::from(png(100, 100, [0, 0, 0]));
        handler
            .replace_part(&LocalId::new("page-1"), redacted)
            .expect("page-1 accepted");

        // Encode reflattens the page: the original page content is gone.
        let out = Handler::encode(&handler).unwrap();
        let bytes = ContentData::to_bytes(&out);
        assert!(
            !String::from_utf8_lossy(&bytes).contains("SCANNED-PAGE-MARKER"),
            "original scanned page content survived reflatten"
        );
        // The output still opens as a one-page PDF.
        assert_eq!(Pdf::open(&bytes).unwrap().inspect().unwrap().page_count, 1);
    }

    #[tokio::test]
    async fn rejects_a_page_id_the_container_did_not_surface() {
        let mut handler =
            PdfHandler::text_auto(scanned_doc(), Vec::<PdfPage>::new(), BTreeMap::new());
        // No scanned pages were surfaced, so `page-1` is not accepted.
        let err = handler
            .replace_part(&LocalId::new("page-1"), Bytes::from_static(b"x"))
            .unwrap_err();
        assert_eq!(err.kind(), elide_core::ErrorKind::MalformedInput);
    }
}
