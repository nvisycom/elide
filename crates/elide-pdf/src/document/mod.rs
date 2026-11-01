//! [`Pdf`]: the opened-document orchestrator.
//!
//! A `Pdf` is a thin wrapper over a `Store` (the data holder: object graph +
//! source bytes + limits). Its methods are the crate's public capability API,
//! extract text, sanitize, redact, render, each delegating to the capability's
//! own module ([`extract`](crate::extract), [`sanitize`](crate::sanitize),
//! [`text`], [`image`],
//! [`attachment`], and `render` behind the `render`
//! feature). A redaction is applied in one pass over one copy of the document
//! and saved once ([`Redaction`]). The container holds no capability logic
//! itself.

mod data;

use std::collections::HashSet;
use std::num::NonZeroUsize;

use bytes::Bytes;
use elide_codec::StrippedContent;
use elide_core::Result;
use lopdf::ObjectId;

pub(crate) use self::data::Store;
use crate::attachment::{self, Attachment, AttachmentId, AttachmentReplacement};
use crate::extract::Extraction;
use crate::image::{self, EmbeddingKind, ImageId, ImageReplacement};
use crate::lopdf_ext::DocumentExt;
#[cfg(feature = "render")]
use crate::render::{self, PageObservation, PageReplacement, RenderedPage};
use crate::sanitize::SanitizeDocumentExt;
use crate::text::{self, Detection};

/// An opened PDF document: parsed once, then reused across operations.
///
/// Open with [`open`](Pdf::open), then extract its text
/// ([`extract`](Pdf::extract)), sanitize it ([`sanitize`](Pdf::sanitize)), or
/// redact it: [`apply`](Pdf::apply) deletes glyphs keeping a selectable layer
/// and writes redacted images, pages, and attachments back, and with the
/// `render` feature [`redact_raster`](Pdf::redact_raster) flattens pages to
/// images.
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
    /// exceeds [`MAX_DOCUMENT_BYTES`](Store::MAX_DOCUMENT_BYTES),
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the
    /// bytes are not a readable PDF, or
    /// [`ErrorKind::CapabilityUnavailable`](crate::ErrorKind::CapabilityUnavailable)
    /// if it is encrypted with a user password.
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
    /// [`apply`](Pdf::apply) expects in its [`Detection`]s, since both
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
        let mut doc = self.0.clone_doc();
        let stripped = doc.sanitize()?;
        if stripped.is_empty() {
            return Ok(None);
        }
        Ok(Some((doc.save_pruned()?, stripped)))
    }

    /// Check that the document carries no hidden content: what the codec runs
    /// on an encoded PDF before returning it.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Sanitization`](crate::ErrorKind::Sanitization) naming what
    /// survived.
    pub fn check_sanitized(&self) -> Result<()> {
        self.0.doc().check_sanitized()
    }

    /// The embedded image `id` of `kind`, stored as `bytes`, as the image
    /// pipeline reads it, with its file-extension hint (see
    /// [`image::surface`]).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if raw
    /// samples the sanitizer kept cannot be decoded.
    pub(crate) fn surface_image(
        &self,
        id: ImageId,
        kind: EmbeddingKind,
        bytes: Bytes,
    ) -> Result<Option<(Bytes, &'static str)>> {
        image::surface(self.0.doc(), id, kind, bytes)
    }

    /// The document with `redaction` applied, every step on one copy of the
    /// document and saved once: the glyphs of the detected text deleted, the
    /// redacted images and scanned pages written back, the unreadable ones
    /// removed or cleared, and the attachments replaced or dropped.
    ///
    /// # Errors
    ///
    /// As each step: [`ErrorKind::Redaction`](crate::ErrorKind::Redaction) if a
    /// replacement or removal names nothing in the document, or a page draws
    /// text with an undecodable font;
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if a
    /// content stream cannot be rewritten or the document cannot be saved.
    pub(crate) fn apply(&self, redaction: &Redaction) -> Result<Vec<u8>> {
        let mut doc = self.0.clone_doc();
        if !redaction.text.is_empty() {
            text::redact::delete(&self.0, &mut doc, &redaction.text)?;
        }
        image::replace(&mut doc, &redaction.images)?;
        if !redaction.removed_images.is_empty() {
            let removed: HashSet<ObjectId> = redaction
                .removed_images
                .iter()
                .map(|id| id.object())
                .collect();
            image::remove(&mut doc, &removed, false)?;
        }
        #[cfg(feature = "render")]
        {
            render::reflatten(&mut doc, &redaction.pages)?;
            render::clear(&mut doc, &redaction.cleared_pages)?;
        }
        attachment::replace(
            &mut doc,
            &redaction.attachments,
            &redaction.removed_attachments,
        )?;
        doc.save_pruned()
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
        attachment::attachments(&self.0)
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

/// Everything an encode writes into a document, applied by [`Pdf::apply`] in
/// one pass and saved once.
#[derive(Debug, Default)]
pub(crate) struct Redaction {
    /// The detected text spans whose glyphs are deleted.
    pub(crate) text: Vec<Detection>,
    /// Embedded images replaced with their redacted bytes.
    pub(crate) images: Vec<ImageReplacement>,
    /// Embedded images no codec could read, removed.
    pub(crate) removed_images: Vec<ImageId>,
    /// Scanned pages reflattened to their redacted raster.
    #[cfg(feature = "render")]
    pub(crate) pages: Vec<PageReplacement>,
    /// Scanned pages no codec could read, cleared.
    #[cfg(feature = "render")]
    pub(crate) cleared_pages: Vec<u32>,
    /// Attachments replaced with their redacted bytes.
    pub(crate) attachments: Vec<AttachmentReplacement>,
    /// Attachments dropped from the document.
    pub(crate) removed_attachments: Vec<AttachmentId>,
}

impl Redaction {
    /// Whether the redaction changes nothing.
    pub(crate) fn is_empty(&self) -> bool {
        #[cfg(feature = "render")]
        let pages = self.pages.is_empty() && self.cleared_pages.is_empty();
        #[cfg(not(feature = "render"))]
        let pages = true;
        self.text.is_empty()
            && self.images.is_empty()
            && self.removed_images.is_empty()
            && pages
            && self.attachments.is_empty()
            && self.removed_attachments.is_empty()
    }
}
