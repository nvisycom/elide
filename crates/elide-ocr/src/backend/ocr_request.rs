//! [`OcrRequest`]: one per-call OCR request handed to an [`OcrBackend`].
//!
//! [`OcrBackend`]: super::OcrBackend

use elide_core::primitive::LanguageTag;
use uuid::Uuid;

/// One per-call OCR request handed to an [`OcrBackend`].
///
/// Bundles the image bytes with advisory hints (filename, language,
/// correlation id). Borrowed (`OcrRequest<'a>`) so call sites that already
/// own the underlying values hand them through without cloning.
///
/// [`OcrBackend`]: super::OcrBackend
#[derive(Debug, Clone)]
pub struct OcrRequest<'a> {
    /// Raw image bytes (PNG, JPEG, TIFF, …). The backend honours whatever
    /// formats it accepts; returned word boxes refer back into this image.
    pub image: &'a [u8],
    /// Original filename, when known. Some backends use the extension for
    /// MIME-type detection on multipart uploads.
    pub filename: Option<&'a str>,
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
            filename: None,
            language: None,
            correlation_id: None,
        }
    }

    /// Builder-style setter for the original filename.
    #[must_use]
    pub fn with_filename(mut self, filename: &'a str) -> Self {
        self.filename = Some(filename);
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
