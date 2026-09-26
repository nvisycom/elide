//! The OCR enricher building block: image text recognition in JavaScript.
//!
//! `elide-image` ships the OCR contract but no engine; the browser supplies one.
//! [`Enricher::ocr`](super::Enricher::ocr) takes an async JS callback
//! `(image) => Promise<OcrBlock[]>` and wraps it in a [`OcrBackend`] behind an
//! [`OcrEnricher`], so a text recognizer can scan the recognized image text and
//! matched regions are redacted from the pixels.

use elide::enrichment::ocr::{OcrBackend, OcrEnricher, OcrRequest, OcrResponse};
use elide::entity::audit::ModelEvent;
use elide::modality::image::{ImageLocation, LayoutBlock};
use elide::primitive::{BoundingBox, Dimensions, Point};
use elide::{Error, ErrorKind, Result};
use js_sys::{Function, Promise, Uint8Array};
use send_wrapper::SendWrapper;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// One recognized text block the JS callback returns, in image-pixel
/// coordinates.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OcrBlock {
    /// The recognized text of the block.
    text: String,
    /// Left edge of the block's box, in pixels.
    x: f64,
    /// Top edge of the block's box, in pixels.
    y: f64,
    /// Box width, in pixels.
    width: f64,
    /// Box height, in pixels.
    height: f64,
}

/// An [`OcrBackend`] that defers recognition to a JavaScript callback.
///
/// The callback is a `!Send` [`Function`]; [`SendWrapper`] makes it satisfy the
/// `Send + Sync` bound the enricher requires — sound on single-threaded wasm.
struct JsCallbackBackend {
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
impl OcrBackend for JsCallbackBackend {
    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "js-callback-ocr".into(),
            ..Default::default()
        }
    }

    async fn recognize(&self, request: OcrRequest<'_>) -> Result<OcrResponse> {
        let value = self
            .call_js(request.image.to_vec())
            .await
            .map_err(super::js_to_error)?;
        let blocks: Vec<OcrBlock> = serde_wasm_bindgen::from_value(value).map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("OCR callback returned an unreadable value: {e}"),
            )
        })?;
        let blocks = blocks
            .into_iter()
            .map(|b| {
                let region = ImageLocation::new(BoundingBox::from_origin(
                    Point::new(b.x, b.y),
                    Dimensions::new(b.width, b.height),
                ));
                LayoutBlock::new(region, b.text)
            })
            .collect();
        Ok(OcrResponse::new(blocks))
    }
}

/// Build an OCR enricher whose recognition is the JavaScript `callback`.
pub(super) fn build_ocr(
    callback: Function,
) -> std::result::Result<OcrEnricher, crate::error::ElideError> {
    Ok(OcrEnricher::builder()
        .with_name("js-callback-ocr")
        .with_backend(JsCallbackBackend::new(callback))
        .build()?)
}
