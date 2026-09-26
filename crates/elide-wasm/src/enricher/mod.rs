//! The enrichment building blocks: the pre-recognition passes that let a modality
//! be detected at all.
//!
//! An image has no text to scan until OCR reads it; an audio clip has none until
//! STT transcribes it; a language-aware recognizer needs the language detected
//! first. `elide` ships each pass's contract but no model — the browser supplies
//! that through a JS callback ([`Enricher::ocr`], [`Enricher::stt`]) or, for
//! language detection, a built-in pure-Rust pass ([`Enricher::language`]). Each
//! returns an [`Enricher`] handed to [`Analyzer::enrich`](crate::analyzer::Analyzer::enrich).
//!
//! A handle is one opaque class regardless of which enricher it wraps, so a
//! caller can pass a mixed list — a language enricher beside an OCR enricher — to
//! one stage. Which enrichers a stage accepts is a modality question: language
//! detection applies to every text-shaped modality, OCR only to image, STT only
//! to audio. An enricher offered to a stage it does not support fails the stage's
//! build with a [`Configuration`](crate::error::ElideErrorKind::Configuration)
//! error. The published TypeScript brands each handle with its supported
//! modalities so this mismatch is also a compile-time error.

mod lingua;
mod ocr;
mod stt;

use elide::detection::Analyzer;
use elide::enrichment::lingua::LinguaEnricher;
use elide::enrichment::ocr::OcrEnricher;
use elide::enrichment::stt::SttEnricher;
use elide::modality::audio::Audio;
use elide::modality::image::Image;
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use js_sys::Function;
use tsify::Ts;
use wasm_bindgen::JsValue;
use wasm_bindgen::prelude::*;

pub use self::lingua::{LanguagePreset, LanguageSet};
use crate::error::{ElideError, ElideErrorKind};

/// The enrichers a handle may carry, one variant per shipped pass.
enum Kind {
    /// Language detection (built-in), applicable to every text-shaped modality.
    Language(LinguaEnricher),
    /// Image OCR (JS callback), applicable only to the image modality.
    Ocr(OcrEnricher),
    /// Audio STT (JS callback), applicable only to the audio modality.
    Stt(SttEnricher),
}

impl Kind {
    /// The enricher's name, for a mismatch error message.
    fn label(&self) -> &'static str {
        match self {
            Self::Language(_) => "a language",
            Self::Ocr(_) => "an OCR",
            Self::Stt(_) => "an STT",
        }
    }
}

/// A pre-recognition enricher, ready to fold into a modality's analyzer.
///
/// Built with [`Enricher::language`], [`Enricher::ocr`], or [`Enricher::stt`],
/// and consumed by [`Analyzer::enrich`](crate::analyzer::Analyzer::enrich). One
/// class wraps every enricher kind, so a mixed list is one `Enricher[]`; the
/// stage rejects a kind it does not support.
#[wasm_bindgen]
pub struct Enricher(Kind);

#[wasm_bindgen]
impl Enricher {
    /// Build the built-in language-detection enricher over `languages` — a preset
    /// (`"english"`, `"common"`, `"all"`) or an explicit list of BCP-47 tags.
    ///
    /// The resulting [`Enricher`] applies to any text-shaped modality (text,
    /// tabular, image, audio). It runs in wasm; no callback is needed.
    ///
    /// # Errors
    ///
    /// Rejects if `languages` is neither a known preset nor a list of valid
    /// BCP-47 tags.
    #[wasm_bindgen(js_name = language)]
    pub fn language(languages: Ts<LanguageSet>) -> Result<Enricher, ElideError> {
        Ok(Self(Kind::Language(self::lingua::build_language(
            languages,
        )?)))
    }

    /// Build an OCR enricher whose recognition is a JavaScript `callback`.
    ///
    /// The callback is `(image: Uint8Array) => Promise<OcrBlock[]>`, where an
    /// `OcrBlock` is `{ text, x, y, width, height }` in image-pixel coordinates.
    /// It runs on the browser event loop; the enricher awaits it. The resulting
    /// [`Enricher`] applies only to the image modality.
    ///
    /// # Errors
    ///
    /// Propagates a build error from the enricher configuration.
    #[wasm_bindgen(js_name = ocr)]
    pub fn ocr(callback: Function) -> Result<Enricher, ElideError> {
        Ok(Self(Kind::Ocr(self::ocr::build_ocr(callback)?)))
    }

    /// Build an STT enricher whose transcription is a JavaScript `callback`.
    ///
    /// The callback is `(audio: Uint8Array) => Promise<Segment[]>`, where a
    /// `Segment` is `{ text, startMs, endMs }`. It runs on the browser event
    /// loop; the enricher awaits it. The resulting [`Enricher`] applies only to
    /// the audio modality.
    ///
    /// # Errors
    ///
    /// Propagates a build error from the enricher configuration.
    #[wasm_bindgen(js_name = stt)]
    pub fn stt(callback: Function) -> Result<Enricher, ElideError> {
        Ok(Self(Kind::Stt(self::stt::build_stt(callback)?)))
    }
}

impl Enricher {
    /// The error for an enricher offered to a stage that cannot run it.
    fn mismatch(label: &str, modality: &str) -> ElideError {
        ElideError::new(
            ElideErrorKind::Configuration,
            format!("{label} enricher does not apply to the {modality} modality"),
        )
    }

    /// Fold this enricher into a [`Text`] analyzer; only language detection
    /// applies.
    pub(crate) fn apply_text(self, analyzer: Analyzer<Text>) -> Result<Analyzer<Text>, ElideError> {
        match self.0 {
            Kind::Language(e) => Ok(analyzer.with_enricher(e)),
            other => Err(Self::mismatch(other.label(), "text")),
        }
    }

    /// Fold this enricher into a [`Tabular`] analyzer; only language detection
    /// applies.
    pub(crate) fn apply_tabular(
        self,
        analyzer: Analyzer<Tabular>,
    ) -> Result<Analyzer<Tabular>, ElideError> {
        match self.0 {
            Kind::Language(e) => Ok(analyzer.with_enricher(e)),
            other => Err(Self::mismatch(other.label(), "tabular")),
        }
    }

    /// Fold this enricher into an [`Image`] analyzer; language detection and OCR
    /// apply.
    pub(crate) fn apply_image(
        self,
        analyzer: Analyzer<Image>,
    ) -> Result<Analyzer<Image>, ElideError> {
        match self.0 {
            Kind::Language(e) => Ok(analyzer.with_enricher(e)),
            Kind::Ocr(e) => Ok(analyzer.with_enricher(e)),
            other => Err(Self::mismatch(other.label(), "image")),
        }
    }

    /// Fold this enricher into an [`Audio`] analyzer; language detection and STT
    /// apply.
    pub(crate) fn apply_audio(
        self,
        analyzer: Analyzer<Audio>,
    ) -> Result<Analyzer<Audio>, ElideError> {
        match self.0 {
            Kind::Language(e) => Ok(analyzer.with_enricher(e)),
            Kind::Stt(e) => Ok(analyzer.with_enricher(e)),
            other => Err(Self::mismatch(other.label(), "audio")),
        }
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
