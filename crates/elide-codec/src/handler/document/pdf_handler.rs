//! PDF handler: adapts the standalone [`elide_pdf`] engine to the codec's
//! [`Handler`] contract.
//!
//! Each page's text is streamed as one [`Chunk`]; a redaction on that chunk is
//! recorded and applied on [`encode`] per the handler's [`RedactMode`]:
//!
//! - **Glyph deletion** (default, pure-Rust): the detected glyphs are deleted
//!   from the content streams and annotations/metadata stripped, keeping a
//!   selectable text layer with the detected spans gone
//!   ([`Pdf::redact_text`](elide_pdf::Pdf::redact_text)).
//! - **Raster** (feature `pdf-render`, [`RasterMode::Always`]): the page text
//!   comes from [`Pdf::observe`](elide_pdf::Pdf::observe) alongside its glyph
//!   geometry, so a redaction's span maps to pixel boxes; encode fills them and
//!   emits a fresh image-only PDF — the text layer is gone entirely.
//!
//! [`encode`]: Handler::encode
//! [`RasterMode::Always`]: super::RasterMode::Always

use bytes::Bytes;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::operator::Redactions;
use elide_core::{Error, ErrorKind, Result};
use elide_pdf::Pdf;
#[cfg(feature = "internal_image")]
use elide_pdf::extract::{EmbeddingKind, ImageId};
use elide_pdf::redact::Detection;
#[cfg(feature = "internal_image")]
use elide_pdf::redact::ImageReplacement;
#[cfg(feature = "pdf-render")]
use elide_pdf::render::{Detection as RasterDetection, PageObservation};

use super::PdfLoader;
#[cfg(feature = "pdf-render")]
use super::RasterMode;
use crate::codec::{Container, Part, PartId};
use crate::content::ContentData;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the PDF codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pdf");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// Decodes on the glyph-deletion redaction path. To flatten pages to images
/// instead, build the format with [`format_with`] and [`RasterMode::Always`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PdfLoader::new())
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
#[cfg(feature = "pdf-render")]
#[cfg_attr(docsrs, doc(cfg(feature = "pdf-render")))]
pub fn format_with(raster: RasterMode) -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PdfLoader::with_raster(raster))
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
    /// Glyph deletion via [`Pdf::redact_text`](elide_pdf::Pdf::redact_text): the
    /// detected glyphs are removed and annotations/metadata stripped, keeping a
    /// selectable text layer. The default pure-Rust redaction path.
    #[default]
    GlyphDelete,
    /// Raster redaction (feature `pdf-render`): the page text comes from
    /// [`Pdf::observe`](elide_pdf::Pdf::observe), so a redaction's span maps
    /// directly to glyph pixel boxes; encode fills them and emits a fresh
    /// image-only PDF.
    #[cfg(feature = "pdf-render")]
    Raster {
        /// Per-page observations (text, glyph boxes, pixels) from `observe`.
        observations: Vec<PageObservation>,
        /// Recorded pixel-span detections, applied at encode.
        detections: Vec<RasterDetection>,
    },
}

impl RedactMode {
    /// Whether redaction flattens pages to images (raster) rather than deleting
    /// glyphs in place.
    fn is_raster(&self) -> bool {
        !matches!(self, RedactMode::GlyphDelete)
    }
}

/// PDF text handler backed by [`elide_pdf`].
///
/// Streams each page's text as a chunk, records per-page redactions, and on
/// [`encode`](Handler::encode) applies them per its [`RedactMode`].
#[derive(Debug, Default)]
pub(crate) struct PdfHandler {
    /// The original document bytes, retained so [`elide_pdf`] re-serialises from
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
    #[cfg(feature = "internal_image")]
    pub(crate) redactable_image_ids: std::collections::BTreeSet<ImageId>,
    /// Redacted replacement images, keyed by their XObject id, filled through
    /// the [`Container`] surface and applied on encode. Only meaningful with an
    /// image codec (`internal_image`) able to redact the surfaced images.
    #[cfg(feature = "internal_image")]
    pub(crate) image_replacements: std::collections::HashMap<ImageId, Bytes>,
    /// How recorded redactions are applied on encode.
    pub(crate) mode: RedactMode,
}

impl PdfHandler {
    /// A glyph-deletion handler over the extracted `pages`: redaction deletes
    /// the detected glyphs, keeping a selectable text layer, and strips
    /// annotations and metadata. The default (pure-Rust) redaction path.
    pub(crate) fn text(document: Bytes, pages: Vec<PdfPage>) -> Self {
        Self {
            #[cfg(feature = "internal_image")]
            redactable_image_ids: redactable_image_ids(&document),
            document,
            pages,
            mode: RedactMode::GlyphDelete,
            ..Self::default()
        }
    }

    /// A raster-redaction handler: `pages` carry the observation text (offsets
    /// into `observations`' glyphs), redacted by pixel fill on encode.
    #[cfg(feature = "pdf-render")]
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
                #[cfg(feature = "internal_image")]
                let has_images = !self.image_replacements.is_empty();
                #[cfg(not(feature = "internal_image"))]
                let has_images = false;
                if self.deletions.is_empty() && !has_images {
                    return Ok(ContentData::new(self.document.clone()));
                }

                // Delete the detected glyphs and strip annotations/metadata,
                // keeping a selectable text layer. (`mut` is used only when the
                // image fold below is compiled in.)
                #[cfg_attr(not(feature = "internal_image"), allow(unused_mut))]
                let mut out = Pdf::open(&self.document)
                    .and_then(|pdf| pdf.redact_text(&self.deletions))
                    .map_err(pdf_error)?;

                // Then fold in any redacted embedded images.
                #[cfg(feature = "internal_image")]
                if has_images {
                    let replacements: Vec<ImageReplacement> = self
                        .image_replacements
                        .iter()
                        .map(|(&id, bytes)| ImageReplacement {
                            id,
                            image: bytes.to_vec(),
                        })
                        .collect();
                    out = Pdf::open(&out)
                        .and_then(|pdf| pdf.redact_images(&replacements))
                        .map_err(pdf_error)?;
                }

                Ok(ContentData::new(Bytes::from(out)))
            }
            #[cfg(feature = "pdf-render")]
            RedactMode::Raster {
                observations,
                detections,
            } => {
                if detections.is_empty() {
                    return Ok(ContentData::new(self.document.clone()));
                }
                // Fill the detected glyph boxes and emit a fresh image-only PDF
                // — the strong redaction guarantee. Black fill.
                let (out, _certificate) = Pdf::open(&self.document)
                    .and_then(|pdf| pdf.redact_raster(observations.clone(), detections, [0, 0, 0]))
                    .map_err(pdf_error)?;
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
        for (location, _replacement) in redactions.into_iter() {
            // Resolve the redaction to the index of the page it falls on and its
            // byte range within that page's text. Both modes address glyphs by
            // span, not replacement text; the span is measured per mode below
            // (char offsets for glyph deletion, UTF-16 for raster) so only the
            // counts a mode needs are computed.
            let Some((page_idx, local, local_end)) =
                self.pages.iter().enumerate().find_map(|(idx, page)| {
                    (location.start >= page.start && location.start < page.start + page.text.len())
                        .then(|| {
                            let local = location.start - page.start;
                            let local_end = location.end - page.start;
                            (idx, local, local_end)
                        })
                })
            else {
                continue;
            };
            let page = &self.pages[page_idx];
            if page.text.get(local..local_end).is_none() {
                continue; // range not on a char boundary
            }
            let page_number = page.number;

            // Measure the span for the active mode only, then drop the page
            // borrow before recording (so the sink can be borrowed mutably).
            if self.mode.is_raster() {
                #[cfg(feature = "pdf-render")]
                {
                    let start = page.text[..local].encode_utf16().count() as u32;
                    let end = page.text[..local_end].encode_utf16().count() as u32;
                    if let RedactMode::Raster { detections, .. } = &mut self.mode {
                        detections.push(RasterDetection::new(page_number, start, end));
                    }
                }
            } else {
                let start = page.text[..local].chars().count();
                let end = page.text[..local_end].chars().count();
                self.deletions.push(Detection::new(page_number, start, end));
            }
        }
        Ok(())
    }
}

impl Container for PdfHandler {
    /// Surface each redactable embedded image XObject as a [`Part`] for the
    /// image pipeline.
    ///
    /// Only with an image codec (`internal_image`) able to decode and redact the
    /// images, and only on the glyph-deletion path — the raster path flattens
    /// every page to an image, so surfacing images for separate redaction would
    /// be redundant. Only images whose bytes are a decodable file are surfaced
    /// (see [`embedding_hint`]).
    fn parts(&self) -> Vec<Part> {
        #[cfg(feature = "internal_image")]
        {
            // In raster mode the whole page is flattened, so per-image redaction
            // is redundant — surface nothing.
            #[cfg(feature = "pdf-render")]
            if matches!(self.mode, RedactMode::Raster { .. }) {
                return Vec::new();
            }
            let Ok(pdf) = Pdf::open(&self.document) else {
                return Vec::new();
            };
            pdf.extract()
                .embeddings
                .into_iter()
                .filter_map(|embedding| {
                    let hint = embedding_hint(embedding.kind)?;
                    Some(Part {
                        id: image_part_id(embedding.id).into(),
                        bytes: embedding.bytes,
                        hint: hint.to_string(),
                    })
                })
                .collect()
        }
        #[cfg(not(feature = "internal_image"))]
        Vec::new()
    }

    fn replace_part(&mut self, id: &PartId, bytes: Bytes) -> Result<()> {
        #[cfg(feature = "internal_image")]
        {
            // Accept only ids naming an image the container surfaced, validated
            // against the cached id set (no re-extraction per call).
            let image_id = parse_image_part_id(id.as_str())
                .filter(|id| self.redactable_image_ids.contains(id));
            if let Some(image_id) = image_id {
                self.image_replacements.insert(image_id, bytes);
                return Ok(());
            }
        }
        #[cfg(not(feature = "internal_image"))]
        let _ = bytes;
        Err(Error::new(
            ErrorKind::MalformedInput,
            format!("pdf replace_part: `{id}` is not a redactable embedded image"),
        ))
    }
}

/// The ids of every embedded image whose bytes are a decodable file (see
/// [`embedding_hint`]) — the set the container will surface and accept back.
#[cfg(feature = "internal_image")]
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

/// The [`PartId`] string for an image XObject: `"img-{number}-{generation}"`.
#[cfg(feature = "internal_image")]
fn image_part_id(id: ImageId) -> String {
    format!("img-{}-{}", id.number, id.generation)
}

/// Parse an image [`PartId`] string back into an [`ImageId`](elide_pdf::extract::ImageId).
#[cfg(feature = "internal_image")]
fn parse_image_part_id(s: &str) -> Option<ImageId> {
    let rest = s.strip_prefix("img-")?;
    let (number, generation) = rest.split_once('-')?;
    Some(ImageId::new(number.parse().ok()?, generation.parse().ok()?))
}

/// A filename-extension hint for an embedded image whose *raw stream bytes* are
/// a self-contained image file the orchestrator can decode, or `None` when they
/// are not.
///
/// An [`Embedding`](elide_pdf::extract::Embedding)'s bytes are the raw XObject
/// stream. For a JPEG (`DCTDecode`) or JPEG 2000 (`JPXDecode`) image those bytes
/// *are* a standalone `.jpg`/`.jp2` file that decodes directly. For the other
/// kinds — raw/`FlateDecode` samples, CCITT fax, JBIG2 — the bytes are filter-
/// specific pixel data that only means anything alongside the XObject's
/// dictionary, so they are **not** a decodable file. Surfacing those with a
/// bogus extension would have the pipeline fail to decode and silently skip
/// them; returning `None` keeps them out of the container entirely, so their
/// non-redaction is an explicit (currently unsupported) case rather than a
/// silent miss.
#[cfg(feature = "internal_image")]
fn embedding_hint(kind: EmbeddingKind) -> Option<&'static str> {
    match kind {
        EmbeddingKind::Jpeg => Some("jpg"),
        EmbeddingKind::Jpeg2000 => Some("jp2"),
        // Raw/Flate/CCITT/JBIG2: the stream bytes are not a self-contained
        // image file. Not surfaced (redacting these is not yet supported).
        EmbeddingKind::CcittFax | EmbeddingKind::Jbig2 | EmbeddingKind::Raw => None,
        _ => None,
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

#[cfg(all(test, feature = "internal_image"))]
mod tests {
    use bytes::Bytes;
    use elide_core::ErrorKind;
    use elide_pdf::Pdf;
    use elide_pdf::extract::ImageId;
    use lopdf::content::{Content, Operation};
    use lopdf::{Dictionary, Document, Object, Stream, dictionary};

    use super::{FORMAT_ID, PdfHandler, PdfPage, image_part_id, parse_image_part_id};
    use crate::Handler;
    use crate::codec::{Container, PartId};

    /// A JPEG-encoded image of a solid colour, as bytes (a self-contained
    /// `.jpg` file, so the container surfaces it — see `embedding_hint`).
    fn jpeg(rgb: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb(rgb));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }

    /// A one-page PDF with a single embedded JPEG (`DCTDecode`) image XObject —
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
            .replace_part(
                &PartId::from("img-9999-0".to_string()),
                Bytes::from(black_png()),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedInput);
    }
}
