//! [`OcrEnricher`]: OCR an image and stamp the recognized text onto the
//! call so the text recognizers can read it.
//!
//! The OCR counterpart to language detection: it produces no entities, it
//! *enriches*. On each call it OCRs the [`ImageData`] bytes through its OCR
//! backend and stamps the resulting [`Layout`] onto the call as
//! `Image`'s [`artifact`]. Recognizers running afterward read the OCR text and
//! resolve each match back to the image region it covers (see [`Image`]'s
//! [`TextRecognizable`] impl).
//!
//! [`ImageData`]: crate::modality::ImageData
//! [`artifact`]: elide_core::recognition::Subject::artifact
//! [`Image`]: crate::modality::Image
//! [`TextRecognizable`]: elide_core::modality::TextRecognizable

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::enrichment::Enricher;
use elide_core::primitive::ComponentId;
use elide_core::recognition::{RecognizerContext, Subject};
use hipstr::HipStr;

#[cfg(any(test, feature = "mocks"))]
use super::MockBackend;
use super::{OcrRequest, OcrResponse};
use crate::modality::{Image, Layout};

/// An [`Enricher<Image>`] that OCRs the image, generic over its OCR backend `B`
/// — any [`Backend`] whose request is [`OcrRequest`] and whose response is
/// [`OcrResponse`].
///
/// Stamps the resulting [`Layout`] onto the call's artifact. Registered on an
/// `Analyzer<Image>` ahead of its recognizers, the same way a language detector
/// is registered on a text analyzer. `B` may be a
/// `Metered` wrapper.
#[derive(Clone)]
pub struct OcrEnricher<B = ()> {
    /// Optional caller-chosen name, surfaced as this enricher's id so a caller
    /// running more than one OCR enricher can tell them apart. `None` falls back
    /// to the crate name at [`id`](Enricher::id) time.
    name: Option<HipStr<'static>>,
    /// Backend that OCRs the image.
    backend: B,
}

impl<B> OcrEnricher<B>
where
    B: for<'a> Backend<Request<'a> = OcrRequest<'a>, Response = OcrResponse>,
{
    /// An OCR enricher over `backend`.
    ///
    /// Unnamed by default (its id falls back to the crate name); set a name with
    /// [`with_name`](Self::with_name) when running more than one so their ids
    /// stay distinct.
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            name: None,
            backend,
        }
    }

    /// Set the enricher name, surfaced as its id.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<HipStr<'static>>) -> Self {
        self.name = Some(name.into());
        self
    }
}

#[cfg(any(test, feature = "mocks"))]
impl OcrEnricher<MockBackend> {
    /// An OCR enricher over the no-op [`MockBackend`].
    ///
    /// [`MockBackend`]: super::MockBackend
    #[cfg_attr(docsrs, doc(cfg(feature = "mocks")))]
    #[must_use]
    pub fn mock() -> Self {
        Self::new(MockBackend::new())
    }
}

impl<B> OcrEnricher<B> {
    /// This enricher's name — the one set with
    /// [`with_name`](Self::with_name), or the crate name when unset.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(env!("CARGO_PKG_NAME"))
    }
}

#[async_trait::async_trait]
impl<B> Enricher<Image> for OcrEnricher<B>
where
    B: for<'a> Backend<Request<'a> = OcrRequest<'a>, Response = OcrResponse>,
{
    fn id(&self) -> ComponentId {
        ComponentId::new(self.name().to_owned(), env!("CARGO_PKG_VERSION"))
    }

    async fn enrich(
        &self,
        subject: &mut Subject<Image>,
        ctx: &RecognizerContext<'_, Image>,
    ) -> Result<()> {
        // Already OCR'd (a second enricher pass, or a restored artifact on a
        // re-run): leave it, so re-recognition never re-invokes the model.
        if subject.is_enriched() {
            return Ok(());
        }
        let data = subject.data();
        let request = OcrRequest {
            image: data.source(),
            format: data.format(),
            dimensions: data.dimensions(),
            language: None,
            correlation_id: ctx.correlation_id(),
        };
        let response = self.backend.call(request).await?;
        subject.set_artifact(Layout::new(response.regions));
        Ok(())
    }
}

// These tests build a real decoded [`ImageData`] via `fixtures`, which the
// `fixtures` feature provides (it implies the decoders).
#[cfg(all(test, feature = "fixtures"))]
mod tests {
    use elide_core::backend::Backend;
    use elide_core::entity::audit::ModelEvent;
    use elide_core::modality::TextRecognizable;
    use elide_core::recognition::Scope;

    use super::*;
    use crate::fixtures;
    use crate::modality::{ImageLocation, LayoutRegion};
    use crate::ocr::OcrResponse;
    use crate::primitive::{BoundingBox, Dimensions, Point};

    fn loc(x: f64, y: f64, w: f64, h: f64) -> ImageLocation {
        ImageLocation::new(BoundingBox::from_origin(
            Point::new(x, y),
            Dimensions::new(w, h),
        ))
    }

    /// A fixed two-region OCR result ("hi Alice") the enricher stamps as a
    /// `Layout`.
    fn canned_regions() -> Vec<LayoutRegion> {
        vec![
            LayoutRegion::new(loc(0.0, 0.0, 30.0, 20.0), "hi"),
            LayoutRegion::new(loc(40.0, 0.0, 60.0, 20.0), "Alice"),
        ]
    }

    #[tokio::test]
    async fn enrich_stamps_readable_ocr_text() {
        let enricher = OcrEnricher::new(MockBackend::with(canned_regions())).with_name("ocr");
        // A set name flows through to the id; unnamed falls back to the crate.
        assert_eq!(enricher.id().name, "ocr");
        assert_eq!(
            OcrEnricher::new(MockBackend::new()).id().name,
            env!("CARGO_PKG_NAME")
        );

        let data = fixtures::blank_image_data();
        let scope = Scope::new();
        let ctx = RecognizerContext::new(&scope);
        let mut subject = Subject::new(data);

        enricher.enrich(&mut subject, &ctx).await.unwrap();

        // Recognizers read the OCR text from the call's artifact.
        assert_eq!(
            Image::as_text(subject.data(), subject.artifact()),
            Some("hi Alice")
        );
        // "Alice" is at bytes 3..8; locate resolves it to the word's box.
        let region =
            Image::locate(3..8, subject.data(), subject.artifact()).expect("range resolves");
        assert_eq!(region.bounding_box.min.x, 40.0);
        assert_eq!(region.bounding_box.max.x, 100.0);
    }

    mockall::mock! {
        /// A spy OCR backend whose `call`s are counted and verifiable, for
        /// asserting the enricher's self-skip on a re-run.
        OcrSpy {}

        #[async_trait::async_trait]
        impl Backend for OcrSpy {
            type Request<'a> = OcrRequest<'a>;
            type Response = OcrResponse;
            fn provenance(&self) -> ModelEvent;
            #[mockall::concretize]
            async fn call(&self, request: OcrRequest<'_>) -> Result<OcrResponse>;
        }
    }

    /// The re-run reuse: an enrich over a context already carrying an artifact
    /// (a restored `Layout` from a prior report) skips the backend entirely, so
    /// re-recognition never re-invokes the OCR model.
    #[tokio::test]
    async fn a_present_artifact_skips_the_backend() {
        let mut backend = MockOcrSpy::new();
        backend.expect_provenance().returning(|| ModelEvent {
            name: "spy".into(),
            ..ModelEvent::default()
        });
        // The self-skip, asserted as a call cardinality: recognize fires exactly
        // once across both enrich calls. mockall fails the test on drop if not.
        backend
            .expect_call()
            .times(1)
            .returning(|_| Ok(OcrResponse::new(canned_regions())));

        let enricher = OcrEnricher::new(backend);
        let data = fixtures::blank_image_data();
        let scope = Scope::new();
        let ctx = RecognizerContext::new(&scope);

        // First pass: empty artifact → the backend runs once.
        let mut subject = Subject::new(data.clone());
        enricher.enrich(&mut subject, &ctx).await.unwrap();

        // Re-run: seed the subject with the prior (restored) artifact. The
        // enricher self-skips, recognize is not called again, enforced by the
        // `.times(1)` above, and the seeded OCR text is still readable.
        let restored = subject
            .artifact()
            .cloned()
            .expect("the first pass enriched");
        let mut subject = Subject::new(data).with_artifact(restored);
        enricher.enrich(&mut subject, &ctx).await.unwrap();
        assert_eq!(
            Image::as_text(subject.data(), subject.artifact()),
            Some("hi Alice")
        );
    }

    /// A restored *empty* `Layout`, a payload a prior pass OCR'd to no text,
    /// still counts as enriched, so the backend is not called again. Guarding on
    /// artifact *presence* rather than emptiness is what makes this hold.
    #[tokio::test]
    async fn a_restored_empty_artifact_skips_the_backend() {
        let mut backend = MockOcrSpy::new();
        backend.expect_provenance().returning(|| ModelEvent {
            name: "spy".into(),
            ..ModelEvent::default()
        });
        // The backend must never run: the seeded (empty) artifact is enrichment.
        backend.expect_call().times(0);

        let enricher = OcrEnricher::new(backend);
        let data = fixtures::blank_image_data();
        let scope = Scope::new();
        let ctx = RecognizerContext::new(&scope);

        // Seed an empty Layout, the recorded result of a prior pass that found
        // no text. The enricher must treat it as already-enriched and skip.
        let mut subject = Subject::new(data).with_artifact(Layout::default());
        enricher.enrich(&mut subject, &ctx).await.unwrap();
        // A present-but-empty artifact reads as `Some("")`, not `None`: the image
        // *was* OCR'd (to no text), which is distinct from never-OCR'd.
        assert_eq!(Image::as_text(subject.data(), subject.artifact()), Some(""));
    }
}
