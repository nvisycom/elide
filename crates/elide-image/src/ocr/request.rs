//! [`OcrRequest`]: one per-call OCR request handed to an [`OcrBackend`].
//!
//! [`OcrBackend`]: super::OcrBackend

use elide_core::backend::BackendRequest;
use elide_core::primitive::LanguageTag;
use uuid::Uuid;

use crate::modality::ImageFormat;
use crate::primitive::Dimensions;

/// One per-call OCR request handed to an [`OcrBackend`].
///
/// Bundles the image bytes with the format and pixel dimensions the caller
/// knows from decoding the image, plus optional language and correlation hints.
/// Borrowed (`OcrRequest<'a>`) so call sites that already own the underlying
/// values hand them through without cloning. Constructed as a struct literal —
/// the required fields are always known by the time OCR runs.
///
/// [`OcrBackend`]: super::OcrBackend
#[derive(Debug, Clone)]
pub struct OcrRequest<'a> {
    /// Raw image bytes (PNG, JPEG, TIFF, …). The backend honours whatever
    /// formats it accepts; returned word boxes refer back into this image.
    pub image: &'a [u8],
    /// The image's container format. Known losslessly from ingestion (a
    /// self-describing container had to be decoded to run OCR), so a remote
    /// backend that never decodes the bytes takes it from here rather than
    /// sniffing.
    pub format: ImageFormat,
    /// The image's pixel dimensions. A backend that returns *normalized* (ratio)
    /// boxes denormalizes them against these; without them it would have to
    /// decode the image just to read two integers it was already handed.
    pub dimensions: Dimensions<u32>,
    /// Caller-asserted language. Backends that support per-call language
    /// hinting use this to pick a model variant; others ignore it.
    pub language: Option<&'a LanguageTag>,
    /// Per-call correlation id propagated to remote backends for tracing.
    pub correlation_id: Option<Uuid>,
}

impl BackendRequest for OcrRequest<'_> {}
