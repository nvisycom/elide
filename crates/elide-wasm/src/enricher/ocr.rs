//! The OCR enricher building block: image text recognition in JavaScript.
//!
//! `elide-image` ships the OCR contract but no engine; the browser supplies one.
//! [`Enricher::ocr`](super::Enricher::ocr) takes an async JS callback
//! `(image) => Promise<OcrRegion[]>` and wraps it in an OCR backend behind an
//! [`OcrEnricher`], so a text recognizer can scan the recognized image text and
//! the matched regions are redacted from the pixels.
//!
//! The callback returns a flat list of positioned text regions — a word, or a
//! coarser run — each with its own box, the shape browser OCR engines emit.

use elide::backend::Backend;
use elide::enrichment::ocr::{OcrEnricher, OcrRequest, OcrResponse};
use elide::entity::audit::ModelEvent;
use elide::modality::image::{ImageLocation, LayoutRegion};
use elide::primitive::{BoundingBox, Confidence, Dimensions, Point};
use elide::{Error, ErrorKind, Result};
use js_sys::{Function, Promise, Uint8Array};
use send_wrapper::SendWrapper;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

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
///
/// The callback is a `!Send` [`Function`]; [`SendWrapper`] makes it satisfy the
/// `Send + Sync` bound the enricher requires — sound on single-threaded wasm.
pub(super) struct JsCallbackBackend {
    callback: SendWrapper<Function>,
}

impl JsCallbackBackend {
    fn new(callback: Function) -> Self {
        Self {
            callback: SendWrapper::new(callback),
        }
    }

    /// Invoke the callback with the image bytes and await its promise, keeping
    /// the `!Send` work inside a [`SendWrapper`] future.
    async fn call_js(&self, image: Vec<u8>) -> std::result::Result<JsValue, JsValue> {
        SendWrapper::new(async move {
            let bytes = Uint8Array::from(image.as_slice());
            let ret = self.callback.call1(&JsValue::NULL, &bytes)?;
            JsFuture::from(Promise::resolve(&ret)).await
        })
        .await
    }
}

#[async_trait::async_trait]
impl Backend for JsCallbackBackend {
    type Request<'a> = OcrRequest<'a>;
    type Response = OcrResponse;

    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "js-callback-ocr".into(),
            ..Default::default()
        }
    }

    async fn call(&self, request: OcrRequest<'_>) -> Result<OcrResponse> {
        let value = self
            .call_js(request.image.to_vec())
            .await
            .map_err(super::js_to_error)?;
        let regions: Vec<OcrRegion> = serde_wasm_bindgen::from_value(value).map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("OCR callback returned an unreadable value: {e}"),
            )
        })?;
        let regions = regions
            .into_iter()
            .map(OcrRegion::into_layout_region)
            .collect();
        Ok(OcrResponse::new(regions))
    }
}

/// Build an OCR enricher whose recognition is the JavaScript `callback`.
pub(super) fn build_ocr(callback: Function) -> OcrEnricher<JsCallbackBackend> {
    OcrEnricher::new(JsCallbackBackend::new(callback)).with_name("js-callback-ocr")
}
