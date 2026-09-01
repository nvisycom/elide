//! [`OcrEnricher`]: OCR an image and stamp the recognized text onto the
//! call so the text recognizers can read it.
//!
//! The OCR counterpart to language detection: it produces no entities, it
//! *enriches*. On each call it OCRs the [`ImageData`] bytes through its
//! [`OcrBackend`] and stamps the resulting [`Layout`] onto the call as
//! `Image`'s [`artifact`]. Recognizers running afterward read the OCR text and
//! resolve each match back to the image region it covers (see [`Image`]'s
//! [`TextRecognizable`] impl).
//!
//! [`ImageData`]: elide_core::modality::image::ImageData
//! [`artifact`]: elide_core::recognition::RecognizerContext::artifact
//! [`OcrBackend`]: crate::OcrBackend
//! [`Image`]: elide_core::modality::image::Image
//! [`TextRecognizable`]: elide_core::modality::TextRecognizable

use std::sync::Arc;

use derive_builder::Builder;
use elide_core::modality::image::{Image, ImageData, Layout};
use elide_core::recognition::{Enricher, Enrichment, RecognizerContext, RecognizerId};
use elide_core::{Error, Result};
use hipstr::HipStr;

#[cfg(any(test, feature = "mock"))]
use crate::MockBackend;
use crate::{OcrBackend, OcrRequest};

/// An [`Enricher<Image>`] that OCRs the image.
///
/// Stamps the resulting [`Layout`] onto the call's artifact. Holds an
/// `Arc<dyn OcrBackend>`; cloning shares the backend. Registered on
/// an `Analyzer<Image>` ahead of its recognizers, the same way a language
/// detector is registered on a text analyzer.
///
/// Built the same way as a NER or LLM recognizer —
/// `OcrEnricher::builder().with_name(..).with_backend(..).build()` — so `name`
/// and `backend` are required and construction is uniform across the
/// model-backed components.
#[derive(Clone, Builder)]
#[builder(
    name = "OcrEnricherBuilder",
    pattern = "owned",
    setter(into, prefix = "with"),
    build_fn(error = "Error", name = "try_build", private)
)]
pub struct OcrEnricher {
    /// Caller-chosen name, surfaced as this enricher's usage id so a caller
    /// running more than one OCR enricher can tell them apart. The model it
    /// calls is reported separately, from the backend's provenance.
    name: HipStr<'static>,
    /// Backend that OCRs the image. Required. Set via [`with_backend`], which
    /// accepts any concrete [`OcrBackend`] impl by value and wraps it in `Arc`
    /// internally.
    ///
    /// [`with_backend`]: OcrEnricherBuilder::with_backend
    #[builder(setter(custom))]
    backend: Arc<dyn OcrBackend>,
}

impl OcrEnricher {
    /// Start the chainable builder. `name` and `backend` are required;
    /// calling [`build`] without them returns a validation error.
    ///
    /// [`build`]: OcrEnricherBuilder::build
    #[must_use]
    pub fn builder() -> OcrEnricherBuilder {
        OcrEnricherBuilder::default()
    }

    /// This enricher's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl OcrEnricherBuilder {
    /// Set the [`OcrBackend`] that OCRs the image. Accepts any concrete impl
    /// by value and wraps it in `Arc`. Required: `build` errors when this
    /// hasn't been called.
    #[must_use]
    pub fn with_backend<B: OcrBackend>(mut self, backend: B) -> Self {
        self.backend = Some(Arc::new(backend));
        self
    }

    /// Wire the no-op [`MockBackend`] as this enricher's backend.
    ///
    /// [`MockBackend`]: crate::MockBackend
    #[cfg(any(test, feature = "mock"))]
    #[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
    #[must_use]
    pub fn with_mock_backend(self) -> Self {
        self.with_backend(MockBackend)
    }

    /// Finish the builder. Errors when `name` or `backend` is unset.
    pub fn build(self) -> Result<OcrEnricher> {
        self.try_build()
    }
}

#[async_trait::async_trait]
impl Enricher<Image> for OcrEnricher {
    fn id(&self) -> RecognizerId {
        RecognizerId::new(self.name.clone(), env!("CARGO_PKG_VERSION"))
    }

    async fn enrich(
        &self,
        data: &ImageData,
        ctx: &mut RecognizerContext<'_, Image>,
    ) -> Result<Enrichment> {
        // Already OCR'd (a second enricher pass, or a restored artifact on a
        // re-run): leave it, so re-recognition never re-invokes the model.
        if !ctx.artifact.is_empty() {
            return Ok(Enrichment::none());
        }
        let mut request = OcrRequest::new(&data.bytes);
        if let Some(name) = data.filename.as_deref() {
            request = request.with_filename(name);
        }
        if let Some(id) = ctx.correlation_id() {
            request = request.with_correlation_id(id);
        }
        let response = self.backend.recognize(request).await?;
        ctx.artifact = Layout::new(response.blocks);
        // The OCR model vouches for its own identity; OCR reports no token
        // counts today.
        #[cfg(feature = "usage")]
        return Ok(Enrichment::with_model(self.backend.provenance().into()));
        #[cfg(not(feature = "usage"))]
        Ok(Enrichment::none())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::ModelEvent;
    use elide_core::modality::TextRecognizable;
    use elide_core::modality::image::{ImageLocation, LayoutBlock, LayoutWord};
    use elide_core::primitive::{BoundingBox, Dimensions, Point};
    use elide_core::recognition::Scope;

    use super::*;
    use crate::OcrResponse;

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin_size(Point::new(x, y), w, h))
    }

    /// Backend returning a fixed one-block, two-word OCR result.
    #[derive(Clone)]
    struct CannedBackend;

    #[async_trait::async_trait]
    impl OcrBackend for CannedBackend {
        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "canned".into(),
                ..ModelEvent::default()
            }
        }

        async fn recognize(&self, _request: OcrRequest<'_>) -> Result<OcrResponse> {
            let block = LayoutBlock::new(loc(0.0, 0.0, 100.0, 20.0), "hi Alice").with_words(vec![
                LayoutWord::new(loc(0.0, 0.0, 30.0, 20.0), "hi"),
                LayoutWord::new(loc(40.0, 0.0, 60.0, 20.0), "Alice"),
            ]);
            Ok(OcrResponse::new(vec![block]))
        }
    }

    #[tokio::test]
    async fn enrich_stamps_readable_ocr_text() {
        let enricher = OcrEnricher::builder()
            .with_name("ocr")
            .with_backend(CannedBackend)
            .build()
            .expect("builder succeeds");
        // The usage id carries the caller's name, not a fixed crate string,
        // so two OCR enrichers can be told apart in the usage report.
        assert_eq!(enricher.id().name, "ocr");

        let data = ImageData::new(b"image".to_vec(), Dimensions::new(100, 20));
        let scope = Scope::new();
        let mut ctx = RecognizerContext::new(&scope);

        enricher.enrich(&data, &mut ctx).await.unwrap();

        // Recognizers read the OCR text from the call's artifact.
        assert_eq!(Image::as_text(&data, &ctx.artifact), "hi Alice");
        // "Alice" is at bytes 3..8; locate resolves it to the word's box.
        let region = Image::locate(3..8, &data, &ctx.artifact).expect("range resolves");
        assert_eq!(region.bounding_box.min.x, 40.0);
        assert_eq!(region.bounding_box.max.x, 100.0);
    }

    /// A backend that counts how many times it is asked to OCR.
    #[derive(Clone, Default)]
    struct CountingBackend(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    #[async_trait::async_trait]
    impl OcrBackend for CountingBackend {
        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "counting".into(),
                ..ModelEvent::default()
            }
        }

        async fn recognize(&self, _request: OcrRequest<'_>) -> Result<OcrResponse> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(OcrResponse::new(vec![LayoutBlock::new(
                loc(0.0, 0.0, 100.0, 20.0),
                "hi Alice",
            )]))
        }
    }

    /// The re-run reuse: an enrich over a context already carrying an artifact
    /// (a restored `Layout` from a prior report) skips the backend entirely, so
    /// re-recognition never re-invokes the OCR model.
    #[tokio::test]
    async fn a_present_artifact_skips_the_backend() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let enricher = OcrEnricher::builder()
            .with_name("ocr")
            .with_backend(CountingBackend(calls.clone()))
            .build()
            .expect("builder succeeds");
        let data = ImageData::new(b"image".to_vec(), Dimensions::new(100, 20));
        let scope = Scope::new();

        // First pass: empty artifact → the backend runs once.
        let mut ctx = RecognizerContext::new(&scope);
        enricher.enrich(&data, &mut ctx).await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Re-run: seed the context with the prior (restored) artifact. The
        // enricher self-skips — the backend is not called again — and the
        // seeded OCR text is still readable.
        let restored = ctx.artifact.clone();
        let mut ctx = RecognizerContext::new(&scope).with_artifact(restored);
        enricher.enrich(&data, &mut ctx).await.unwrap();
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a restored artifact must not re-invoke the OCR backend",
        );
        assert_eq!(Image::as_text(&data, &ctx.artifact), "hi Alice");
    }
}
