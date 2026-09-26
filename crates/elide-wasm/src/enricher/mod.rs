//! The enrichment building blocks: the pre-recognition passes that let a modality
//! be detected at all.
//!
//! An image has no text to scan until OCR reads it; an audio clip has none until
//! STT transcribes it; a language-aware recognizer needs the language detected
//! first. `elide` ships each pass's contract but no model — the browser supplies
//! that through a JS callback ([`create_ocr_enricher`], [`create_stt_enricher`])
//! or, for language detection, a built-in pure-Rust pass
//! ([`create_lingua_enricher`]). Each returns a handle folded into the matching
//! modality on the [`PipelineBuilder`](crate::pipeline::PipelineBuilder).

mod lingua;
mod ocr;
mod stt;

use elide::enrichment::lingua::LinguaEnricher;
use elide::enrichment::ocr::OcrEnricher;
use elide::enrichment::stt::SttEnricher;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;

pub use self::lingua::create_lingua_enricher;
pub use self::ocr::create_ocr_enricher;
pub use self::stt::create_stt_enricher;

/// An image enricher (OCR): reads an image's text so recognizers can scan it.
///
/// Opaque and consumed by
/// [`with_image`](crate::pipeline::PipelineBuilder::with_image).
#[wasm_bindgen]
pub struct ImageEnricherHandle(OcrEnricher);

impl ImageEnricherHandle {
    pub(crate) fn new(enricher: OcrEnricher) -> Self {
        Self(enricher)
    }

    /// The OCR enricher, for the image analyzer.
    pub(crate) fn into_enricher(self) -> OcrEnricher {
        self.0
    }
}

/// An audio enricher (STT): transcribes a clip so recognizers can scan it.
///
/// Opaque and consumed by
/// [`with_audio`](crate::pipeline::PipelineBuilder::with_audio).
#[wasm_bindgen]
pub struct AudioEnricherHandle(SttEnricher);

impl AudioEnricherHandle {
    pub(crate) fn new(enricher: SttEnricher) -> Self {
        Self(enricher)
    }

    /// The STT enricher, for the audio analyzer.
    pub(crate) fn into_enricher(self) -> SttEnricher {
        self.0
    }
}

/// A text enricher (language detection): resolves the input's language so
/// language-aware recognizers and policies apply.
///
/// [`LinguaEnricher`] detects over any text-shaped modality, so one handle folds
/// into the text, tabular, or audio stage. Opaque and consumed by the `with_*`
/// method it is given to.
#[wasm_bindgen]
pub struct TextEnricherHandle(LinguaEnricher);

impl TextEnricherHandle {
    pub(crate) fn new(enricher: LinguaEnricher) -> Self {
        Self(enricher)
    }

    /// The language enricher, for a text-shaped analyzer.
    pub(crate) fn into_enricher(self) -> LinguaEnricher {
        self.0
    }
}

/// Turn a `JsValue` thrown or rejected by a callback into the crate error,
/// shared by the OCR and STT backends.
pub(super) fn js_to_error(value: JsValue) -> elide::Error {
    let message = value.as_string().unwrap_or_else(|| {
        js_sys::Reflect::get(&value, &JsValue::from_str("message"))
            .ok()
            .and_then(|m| m.as_string())
            .unwrap_or_else(|| "enricher callback rejected".to_owned())
    });
    elide::Error::new(elide::ErrorKind::Processing, message)
}
