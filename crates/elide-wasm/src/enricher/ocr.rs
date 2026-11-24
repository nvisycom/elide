//! The OCR enricher building block: image text recognition in JavaScript.
//!
//! `elide-image` ships the OCR contract but no engine; the browser supplies one.
//! [`Enricher::ocr`](super::Enricher::ocr) takes a [`Backend`] whose callback is
//! `(image) => Promise<OcrRegion[]>` and builds an [`OcrEnricher`] over it, so
//! a text recognizer can scan the recognized image text and the matched regions
//! are redacted from the pixels.
//!
//! The callback returns a flat list of positioned text regions — a word, or a
//! coarser run — each with its own box, the shape browser OCR engines emit.

use elide::Result;
use elide::backend::Backend as BackendTrait;
use elide::enrichment::ocr::{OcrEnricher, OcrRequest, OcrResponse};
use elide::entity::audit::ModelId;
use elide::modality::image::{ImageLocation, LayoutRegion};
use elide::primitive::{BoundingBox, Confidence, Dimensions, Point};
use js_sys::{Array, Uint8Array};
use serde::Deserialize;
use wasm_bindgen::JsValue;

use crate::backend::{Backend, Composed, JsCallback};

/// The component's name, and the model every response of the callback
/// names.
const NAME: &str = "js-callback-ocr";

/// One recognized text region the JS callback returns: the text, its box (the
/// top-left corner and size, in pixels), and an optional confidence.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OcrRegion {
    /// The recognized text of the region.
    text: String,
    /// Left edge of the box, in pixels.
    x: f64,
    /// Top edge of the box, in pixels.
    y: f64,
    /// Box width, in pixels.
    width: f64,
    /// Box height, in pixels.
    height: f64,
    /// Recognition confidence in `[0, 1]`, when the engine reports it.
    #[serde(default)]
    confidence: Option<f32>,
}

impl OcrRegion {
    /// Build the layout region, mapping the box to an [`ImageLocation`].
    fn into_layout_region(self) -> LayoutRegion {
        let location = ImageLocation::new(BoundingBox::from_origin(
            Point::new(self.x, self.y),
            Dimensions::new(self.width, self.height),
        ));
        let region = LayoutRegion::new(location, self.text);
        match self.confidence {
            Some(score) => region.with_confidence(Confidence::clamped(score)),
            None => region,
        }
    }
}

/// An OCR backend that defers recognition to a JavaScript callback.
pub(crate) struct JsOcrBackend(JsCallback);

#[async_trait::async_trait]
impl BackendTrait for JsOcrBackend {
    type Request<'a> = OcrRequest<'a>;
    type Response = OcrResponse;

    async fn call(&self, request: OcrRequest<'_>) -> Result<OcrResponse> {
        let (image, language) = (request.image, request.language);
        let regions: Vec<OcrRegion> = self
            .0
            .call("OCR", move || {
                let language =
                    language.map_or(JsValue::UNDEFINED, |tag| JsValue::from_str(tag.as_str()));
                Ok(Array::of2(&Uint8Array::from(image), &language))
            })
            .await?;
        let regions = regions
            .into_iter()
            .map(OcrRegion::into_layout_region)
            .collect();
        Ok(OcrResponse::new(ModelId::new(NAME), regions))
    }
}

/// Build an OCR enricher whose recognition is `backend`'s JavaScript callback.
pub(super) fn build_ocr(backend: Backend) -> OcrEnricher<Composed<JsOcrBackend>> {
    OcrEnricher::new(backend.build(&JsOcrBackend)).with_name(NAME)
}
