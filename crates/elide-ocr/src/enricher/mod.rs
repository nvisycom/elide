//! [`OcrEnricher`]: OCR an image and stamp the recognized text onto the
//! call so the text recognizers can read it.
//!
//! The OCR counterpart to language detection: it produces no entities, it
//! *enriches*. On each call it OCRs the [`ImageData`] bytes through its
//! [`OcrBackend`] and inserts the resulting [`Layout`] into the call's
//! [`artifacts`]. Recognizers running afterward read the OCR text and
//! resolve each match back to the image region it covers (see [`Image`]'s
//! [`TextRecognizable`] impl).
//!
//! [`ImageData`]: elide_core::modality::image::ImageData
//! [`artifacts`]: elide_core::recognition::RecognizerContext::artifacts
//! [`OcrBackend`]: crate::OcrBackend
//! [`Image`]: elide_core::modality::image::Image
//! [`TextRecognizable`]: elide_core::modality::TextRecognizable

use std::sync::Arc;

use elide_core::Result;
use elide_core::modality::image::{Image, ImageData, Layout};
use elide_core::recognition::{Enricher, RecognizerContext};

use crate::{OcrBackend, OcrRequest};

/// An [`Enricher<Image>`] that OCRs the image and stamps the [`Layout`]
/// onto the call's artifacts.
///
/// Holds an `Arc<dyn OcrBackend>`; cloning shares the backend. Registered on
/// an `Analyzer<Image>` ahead of its recognizers, the same way a language
/// detector is registered on a text analyzer.
#[derive(Clone)]
pub struct OcrEnricher {
    backend: Arc<dyn OcrBackend>,
}

impl OcrEnricher {
    /// An enricher that OCRs with `backend`.
    pub fn new(backend: impl OcrBackend) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }
}

impl Enricher<Image> for OcrEnricher {
    async fn enrich(&self, data: &ImageData, ctx: &mut RecognizerContext<'_, Image>) -> Result<()> {
        // Already OCR'd (e.g. a second enricher pass): leave it.
        if ctx.artifacts.contains::<Layout>() {
            return Ok(());
        }
        let mut request = OcrRequest::new(&data.bytes);
        if let Some(name) = data.filename.as_deref() {
            request = request.with_filename(name);
        }
        if let Some(id) = ctx.correlation_id() {
            request = request.with_correlation_id(id);
        }
        let response = self.backend.recognize(request).await?;
        ctx.artifacts.insert(Layout::new(response.blocks));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::ModelEvent;
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
        let enricher = OcrEnricher::new(CannedBackend);
        let data = ImageData::new(b"image".to_vec(), Dimensions::new(100, 20));
        let scope = Scope::<Image>::new();
        let mut ctx = RecognizerContext::new(&scope);

        enricher.enrich(&data, &mut ctx).await.unwrap();

        // Recognizers read the OCR text from the call's artifacts.
        assert_eq!(Image::as_text(&data, &ctx.artifacts), "hi Alice");
        // "Alice" is at bytes 3..8; locate resolves it to the word's box.
        let region = Image::locate(3..8, &data, &ctx.artifacts).expect("range resolves");
        assert_eq!(region.bounding_box.min.x, 40.0);
        assert_eq!(region.bounding_box.max.x, 100.0);
    }
}
