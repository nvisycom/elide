//! Backend layer: the [`OcrBackend`] trait and its shipped impls.
//!
//! One trait covers every flavour of OCR engine — hosted document-AI APIs
//! (Google Document AI, Azure, AWS Textract), local engines (Tesseract,
//! PaddleOCR wrappers), and the in-process no-op test stub. Each backend
//! turns a request (image bytes + optional hints) into a response of
//! recognized [`OcrBlock`]s — the core OCR type, so a backend's output
//! drops straight onto the call's artifacts with no remapping. The
//! `mock`-gated `MockBackend` (returns no blocks; test/example stub) ships
//! here; concrete engine backends live downstream.
//!
//! [`OcrBlock`]: elide_core::primitive::OcrBlock

#[cfg(any(test, feature = "mock"))]
mod mock_backend;
mod ocr_request;
mod ocr_response;

use elide_core::Result;
use elide_core::entity::provenance::ModelEvent;

#[cfg(any(test, feature = "mock"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
pub use self::mock_backend::MockBackend;
pub use self::ocr_request::OcrRequest;
pub use self::ocr_response::OcrResponse;

/// Per-call OCR backend.
///
/// Implemented by everything that turns image bytes into recognized text
/// blocks — hosted document-AI clients, local OCR engine wrappers, and the
/// in-process no-op test stub. Each block carries its bounding region and,
/// when the engine emits them, per-word boxes; the recognizer resolves a
/// matched byte range back to the region it covers.
///
/// Confidence values **must** be normalised to `0.0..=1.0` before being
/// placed on a word. Backends whose upstream API uses a different scale
/// convert before returning.
///
/// Object-safe: enrichers hold `Arc<dyn OcrBackend>` and dispatch per call.
#[async_trait::async_trait]
pub trait OcrBackend: Send + Sync + 'static {
    /// Backend identity (model / service name + provenance detail).
    ///
    /// Identifies the actual engine the backend wraps (e.g. `"noop-ocr"`),
    /// stamped into the provenance of every entity detected over the OCR
    /// text so the audit records which OCR pass produced it.
    fn provenance(&self) -> ModelEvent;

    /// Recognize text in `request` into ordered blocks.
    ///
    /// # Errors
    ///
    /// Returns the underlying transport / parse / inference error.
    async fn recognize(&self, request: OcrRequest<'_>) -> Result<OcrResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_empty() {
        let backend = MockBackend;
        let image = vec![0u8; 8];
        let response = backend.recognize(OcrRequest::new(&image)).await.unwrap();
        assert!(response.blocks.is_empty());
    }
}
