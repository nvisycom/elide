//! [`OcrRequest`]: one per-call OCR request handed to an [`OcrBackend`].
//!
//! [`OcrBackend`]: super::OcrBackend

use elide_core::primitive::LanguageTag;
use uuid::Uuid;

use crate::modality::ImageFormat;

/// One per-call OCR request handed to an [`OcrBackend`].
///
/// Bundles the image bytes with advisory hints (format, language, correlation
/// id). Borrowed (`OcrRequest<'a>`) so call sites that already own the
/// underlying values hand them through without cloning.
///
/// [`OcrBackend`]: super::OcrBackend
#[derive(Debug, Clone)]
pub struct OcrRequest<'a> {
    /// Raw image bytes (PNG, JPEG, TIFF, …). The backend honours whatever
    /// formats it accepts; returned word boxes refer back into this image.
    pub image: &'a [u8],
    /// Caller-asserted container format. A remote backend that never decodes
    /// the bytes locally needs an out-of-band format hint (a MIME type or
    /// extension); the caller, which read the source, knows it losslessly,
    /// whereas byte-sniffing downstream can only approximate it. `None` leaves
    /// the backend to sniff.
    pub format: Option<ImageFormat>,
    /// Caller-asserted language. Backends that support per-call language
    /// hinting use this to pick a model variant; others ignore it.
    pub language: Option<&'a LanguageTag>,
    /// Per-call correlation id propagated to remote backends for tracing.
    pub correlation_id: Option<Uuid>,
}

impl<'a> OcrRequest<'a> {
    /// A request over `image` with no advisory hints set.
    pub fn new(image: &'a [u8]) -> Self {
        Self {
            image,
            format: None,
            language: None,
            correlation_id: None,
        }
    }

    /// Builder-style setter for the caller-asserted container format.
    #[must_use]
    pub fn with_format(mut self, format: ImageFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Builder-style setter for the language hint.
    #[must_use]
    pub fn with_language(mut self, language: &'a LanguageTag) -> Self {
        self.language = Some(language);
        self
    }

    /// Builder-style setter for the correlation id.
    #[must_use]
    pub fn with_correlation_id(mut self, correlation_id: Uuid) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }
}
