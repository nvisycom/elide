//! [`Pdf`]: the opened-document orchestrator.
//!
//! A `Pdf` is a thin wrapper over a `Store` (the data holder: object graph +
//! source bytes + limits). Its methods are the crate's public capability API,
//! extract text, inspect structure, redact, render, each delegating to a
//! function in the capability's own module ([`extract`](crate::extract),
//! [`text`](crate::text), [`inspect`](crate::inspect), [`redact`](crate::redact),
//! and `render` behind the `render` feature) that operates over the `Store`. The
//! container holds no capability logic itself.

mod data;
#[cfg(feature = "image")]
mod role;

use std::num::NonZeroUsize;

use elide_codec::StrippedContent;
use elide_core::{Error, ErrorKind, Result};
use lopdf::{Document, Object};

pub(crate) use self::data::Store;
#[cfg(feature = "image")]
pub(crate) use self::role::ObjectRole;
use crate::attachment::{Attachment, AttachmentId, AttachmentReplacement};
use crate::extract::Extraction;
use crate::redact::Detection;
#[cfg(feature = "image")]
use crate::redact::ImageReplacement;
#[cfg(feature = "render")]
use crate::redact::PageReplacement;
#[cfg(feature = "render")]
use crate::render::{PageObservation, RenderedPage};
use crate::sanitize::{check_store, sanitize_store};

/// An opened PDF document: parsed once, then reused across operations.
///
/// Open with [`open`](Pdf::open), then extract its text ([`extract`](Pdf::extract),
/// inspect its structure ([`inspect`](Pdf::inspect),
/// [`verify_flattened`](Pdf::verify_flattened)), or redact it,
/// [`redact_text`](Pdf::redact_text) deletes glyphs keeping a selectable layer,
/// and with the `render` feature `redact_raster` flattens pages to images.
#[derive(Debug, Clone)]
pub struct Pdf(Store);

impl Pdf {
    /// Default bound on a single page's decompressed content, guarding against a
    /// decompression bomb. Override with [`open_with_limit`](Pdf::open_with_limit).
    pub const DEFAULT_MAX_PAGE_BYTES: NonZeroUsize = Store::DEFAULT_MAX_PAGE_BYTES;

    /// Open a PDF from its bytes, using the
    /// [default page bound](Pdf::DEFAULT_MAX_PAGE_BYTES).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](crate::ErrorKind::ResourceLimit) if the input
    /// exceeds [`MAX_DOCUMENT_BYTES`](Pdf::MAX_DOCUMENT_BYTES), or
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the
    /// bytes are not a readable PDF.
    pub fn open(document: &[u8]) -> Result<Self> {
        Self::open_with_limit(document, Self::DEFAULT_MAX_PAGE_BYTES)
    }

    /// Open a PDF with an explicit per-page decompressed-size bound.
    ///
    /// # Errors
    ///
    /// As [`open`](Pdf::open).
    pub fn open_with_limit(document: &[u8], max_page_bytes: NonZeroUsize) -> Result<Self> {
        Store::open(document, max_page_bytes).map(Self)
    }

    /// Extract the document's text and embedded images, with a per-page
    /// [`issue`](crate::extract::Extraction::issues) for any page that yielded no
    /// text (scanned, needs OCR) or could not be decoded, and each readable page's
    /// rendered glyph [`geometry`](crate::text::PageGeometry) from the *same*
    /// content walk (so per-entity page rectangles cost no second walk).
    ///
    /// This is the text surface a caller runs detection over: a
    /// [`Block`](crate::extract::Block)'s character offsets into its text are what
    /// [`redact_text`](Pdf::redact_text) expects in its [`Detection`]s, since both
    /// use the same content walk. The geometry maps a detected character span back
    /// to the page rectangles it covers.
    pub(crate) fn extract(&self) -> (Extraction, Vec<crate::text::PageGeometry>) {
        Extraction::from_store_with_geometry(&self.0)
    }

    /// The document with its hidden content removed, as new bytes with the
    /// record of what was removed, or `None` when it carries none: metadata,
    /// annotations other than internal links, actions and scripts, form values,
    /// the outline and structure tree, optional-content layers, thumbnails, and
    /// every object no longer reachable from the document.
    ///
    /// The codec runs this at decode, so the text extracted for detection is the
    /// text that ships.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the
    /// sanitized document cannot be saved.
    pub fn sanitize(&self) -> Result<Option<(Vec<u8>, StrippedContent)>> {
        sanitize_store(&self.0)
    }

    /// Check that the document carries no hidden content: what the codec runs
    /// on an encoded PDF before returning it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`](crate::ErrorKind::Sanitization) naming what
    /// survived.
    pub fn check_sanitized(&self) -> Result<()> {
        check_store(&self.0)
    }

    /// Every attached file (the `/EmbeddedFiles` name tree), decoded, in tree
    /// order.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the
    /// tree or a file cannot be read, or
    /// [`ErrorKind::ResourceLimit`](crate::ErrorKind::ResourceLimit) if a file
    /// decompresses past the bound.
    pub(crate) fn attachments(&self) -> Result<Vec<Attachment>> {
        crate::attachment::attachments(&self.0)
    }

    /// Write each replacement's redacted bytes into its attached file and drop
    /// each `removed` attachment, returning the new bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a replacement or
    /// removal names no attachment of the document.
    pub fn redact_attachments(
        &self,
        replacements: &[AttachmentReplacement],
        removed: &[AttachmentId],
    ) -> Result<Vec<u8>> {
        crate::attachment::redact_attachments(&self.0, replacements, removed)
    }

    /// Redact `detections` by deleting the glyphs that drew them, returning the
    /// new bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the
    /// document cannot be read or re-saved, or
    /// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a page draws text
    /// with an undecodable font, refused rather than left in place.
    pub fn redact_text(&self, detections: &[Detection]) -> Result<Vec<u8>> {
        crate::redact::redact_text(&self.0, detections)
    }

    /// Replace embedded image XObjects with redacted images, returning the new
    /// bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a replacement
    /// names a non-image object or an image that cannot be decoded.
    #[cfg(feature = "image")]
    #[cfg_attr(docsrs, doc(cfg(feature = "image")))]
    pub fn redact_images(&self, replacements: &[ImageReplacement]) -> Result<Vec<u8>> {
        crate::redact::redact_images(&self.0, replacements)
    }

    /// Reflatten the named pages, replacing each page's whole content with a
    /// redacted image, returning the new bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a replacement
    /// names a page that does not exist or an undecodable image.
    #[cfg(feature = "render")]
    #[cfg_attr(docsrs, doc(cfg(feature = "render")))]
    pub fn redact_pages(&self, replacements: &[PageReplacement]) -> Result<Vec<u8>> {
        crate::redact::redact_pages(&self.0, replacements)
    }

    /// Render only the 1-based pages in `numbers` at `scale`, keyed by page
    /// number, so a pass needing a few pages does not rasterise the whole
    /// document.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if PDFium
    /// cannot load or render the document (or the native library is unavailable).
    #[cfg(feature = "render")]
    #[cfg_attr(docsrs, doc(cfg(feature = "render")))]
    pub fn render_pages(
        &self,
        numbers: std::collections::BTreeSet<u32>,
        scale: f32,
    ) -> Result<std::collections::BTreeMap<u32, RenderedPage>> {
        crate::render::render_pages(&self.0, numbers, scale)
    }

    /// Observe every page at `scale`: render it to pixels and extract its
    /// text-layer glyphs in rendered-pixel space, the input to raster redaction.
    ///
    /// # Errors
    ///
    /// As [`render_pages`](Pdf::render_pages).
    #[cfg(feature = "render")]
    #[cfg_attr(docsrs, doc(cfg(feature = "render")))]
    pub fn observe(&self, scale: f32) -> Result<Vec<PageObservation>> {
        crate::render::observe(&self.0, scale)
    }

    /// Redact `detections` by overwriting their pixels and emitting a fresh
    /// image-only PDF (flatten), with a provenance certificate.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) for a
    /// malformed observation, or
    /// [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if the redacted PDF
    /// cannot be emitted.
    #[cfg(feature = "render")]
    #[cfg_attr(docsrs, doc(cfg(feature = "render")))]
    pub fn redact_raster(
        &self,
        pages: &[PageObservation],
        detections: &[Detection],
        fill: [u8; 3],
    ) -> Result<(Vec<u8>, crate::render::Certificate)> {
        crate::render::redact_raster(&self.0, pages, detections, fill)
    }
}

/// Save `doc`, first dropping every object it no longer reaches: a superseded
/// content stream, a replaced image's mask, a stale cross-reference stream. An
/// unreachable object is still serialized by `save_to`, so without the prune a
/// redacted original would survive in the bytes.
///
/// # Errors
///
/// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the
/// document cannot be serialized.
pub(crate) fn save(doc: &mut Document) -> Result<Vec<u8>> {
    doc.objects.retain(|_, object| !is_xref_stream(object));
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| {
        Error::new(
            ErrorKind::MalformedInput,
            format!("could not save PDF: {e}"),
        )
    })?;
    Ok(out)
}

/// Whether `object` is a cross-reference stream: file structure the next save
/// rewrites, never content.
pub(crate) fn is_xref_stream(object: &Object) -> bool {
    matches!(object, Object::Stream(stream) if stream.dict.has_type(b"XRef"))
}
