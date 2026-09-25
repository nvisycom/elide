//! OCR: recognize the text laid out in an image.
//!
//! Enriches the call with a [`Layout`](crate::modality::Layout).
//!
//! The [`OcrBackend`] trait covers every OCR engine, hosted document-AI APIs
//! (Google Document AI, Azure, AWS Textract), local engines (Tesseract, PaddleOCR
//! wrappers), and the in-process no-op test stub. Each backend turns a request
//! (image bytes + optional hints) into a response of recognized
//! [`LayoutBlock`](crate::modality::LayoutBlock)s, so its output drops straight
//! onto the call's artifacts with no remapping. The [`OcrEnricher`] drives a
//! backend per call and stamps the recognized [`Layout`](crate::modality::Layout)
//! onto the image so a recognizer can read it. The pure-Rust `OcrsBackend` is
//! behind the `ocrs` feature; the no-op `MockBackend` behind `test-utils`.

mod enricher;
#[cfg(any(test, feature = "test-utils"))]
mod mock;
#[cfg(feature = "ocrs")]
mod ocrs;
mod request;
mod response;

use elide_core::Result;
use elide_core::entity::audit::ModelEvent;

pub use self::enricher::{OcrEnricher, OcrEnricherBuilder};
#[cfg(any(test, feature = "test-utils"))]
#[cfg_attr(docsrs, doc(cfg(feature = "test-utils")))]
pub use self::mock::MockBackend;
#[cfg(feature = "ocrs")]
#[cfg_attr(docsrs, doc(cfg(feature = "ocrs")))]
pub use self::ocrs::{OCRS_MODELS_DIR_ENV, OcrsBackend};
pub use self::request::OcrRequest;
pub use self::response::OcrResponse;

/// Per-call OCR backend.
///
/// Implemented by everything that turns image bytes into recognized text
/// blocks, hosted document-AI clients, local OCR engine wrappers, and the
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

    /// Batched recognize. Defaults to a sequential fan-out;
    /// backends with native batching should override.
    ///
    /// # Errors
    ///
    /// Returns the first error encountered.
    async fn recognize_batch(&self, requests: &[OcrRequest<'_>]) -> Result<Vec<OcrResponse>> {
        let mut out = Vec::with_capacity(requests.len());
        for req in requests {
            out.push(self.recognize(req.clone()).await?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_empty() {
        let backend = MockBackend::new();
        let image = vec![0u8; 8];
        let response = backend.recognize(OcrRequest::new(&image)).await.unwrap();
        assert!(response.blocks.is_empty());
    }
}
