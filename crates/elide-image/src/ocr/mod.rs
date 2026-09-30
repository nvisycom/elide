//! OCR: recognize the text laid out in an image.
//!
//! Enriches the call with a [`Layout`](crate::modality::Layout).
//!
//! An OCR backend is a [`Backend`](elide_core::backend::Backend) whose request is
//! [`OcrRequest`] and whose response is [`OcrResponse`]. It covers every OCR
//! engine: hosted document-AI APIs (Google Document AI, Azure, AWS Textract),
//! local engines (Tesseract, PaddleOCR wrappers), and the in-process no-op test
//! stub. Each turns a request (image bytes + optional hints) into a response of
//! recognized [`LayoutRegion`](crate::modality::LayoutRegion)s, so its output
//! drops straight onto the call's artifacts with no remapping. The
//! [`OcrEnricher`] drives a backend per call and stamps the recognized
//! [`Layout`](crate::modality::Layout) onto the image so a recognizer can read it.
//! The pure-Rust `OcrsBackend` is behind the `ocrs` feature; the no-op
//! `MockBackend` behind `mocks`.
//!
//! Confidence values **must** be normalised to `0.0..=1.0` before being placed on
//! a word; a backend whose upstream API uses a different scale converts before
//! returning.

mod enricher;
#[cfg(any(test, feature = "mocks"))]
mod mock;
#[cfg(feature = "ocrs")]
mod ocrs;
mod request;
mod response;

pub use self::enricher::OcrEnricher;
#[cfg(any(test, feature = "mocks"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
pub use self::mock::MockBackend;
#[cfg(feature = "ocrs")]
#[cfg_attr(docsrs, doc(cfg(feature = "ocrs")))]
pub use self::ocrs::{OCRS_MODELS_DIR_ENV, OcrsBackend};
pub use self::request::OcrRequest;
pub use self::response::OcrResponse;

#[cfg(test)]
mod tests {
    use elide_core::backend::Backend;

    use super::*;
    use crate::modality::ImageFormat;
    use crate::primitive::Dimensions;

    #[tokio::test]
    async fn mock_returns_empty() {
        let backend = MockBackend::new();
        let image = vec![0u8; 8];
        let request = OcrRequest {
            image: &image,
            format: ImageFormat::Png,
            dimensions: Dimensions::new(1, 1),
            language: None,
            correlation_id: None,
        };
        let response = Backend::call(&backend, request).await.unwrap();
        assert!(response.regions.is_empty());
    }
}
