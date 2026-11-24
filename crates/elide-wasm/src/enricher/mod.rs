//! The enrichment building blocks: passes that run before recognition.
//!
//! They let a modality be detected at all.
//!
//! An image has no text to scan until OCR reads it; an audio clip has none until
//! STT transcribes it; a language-aware recognizer needs the language detected
//! first. `elide` ships each pass's contract but no model — the browser supplies
//! that through a JS-callback [`Backend`] ([`Enricher::language`],
//! [`Enricher::ocr`], [`Enricher::stt`]). Each returns an [`Enricher`] handed to
//! [`Analyzer::enrich`](crate::analyzer::Analyzer::enrich).
//!
//! A handle is one opaque class regardless of which enricher it wraps, so a
//! caller can pass a mixed list — a language enricher beside an OCR enricher —
//! to one stage. Which enrichers a stage accepts is a modality question:
//! language detection applies to every stage, OCR only to image, STT only to
//! audio. An enricher offered to a stage it does not support fails the stage's
//! build with a [`Configuration`](crate::error::ElideErrorKind::Configuration)
//! error. The published TypeScript brands each handle with its supported
//! modalities so this mismatch is also a compile-time error.

mod language;
mod ocr;
mod stt;

use elide::detection::Analyzer;
use elide::enrichment::ocr::OcrEnricher;
use elide::enrichment::stt::SttEnricher;
use elide::modality::audio::Audio;
use elide::modality::image::Image;
use elide::modality::tabular::Tabular;
use elide::modality::text::Text;
use wasm_bindgen::prelude::*;

use crate::backend::{Backend, Composed};
use crate::error::ElideError;

/// The enrichers a handle may carry, one variant per shipped pass.
enum Kind {
    /// Language detection (JS callback), applicable to every stage.
    Language(self::language::LanguageEnricher<Composed<self::language::JsLanguageBackend>>),
    /// Image OCR (JS callback), applicable only to the image modality.
    Ocr(OcrEnricher<Composed<self::ocr::JsOcrBackend>>),
    /// Audio STT (JS callback), applicable only to the audio modality.
    Stt(SttEnricher<Composed<self::stt::JsSttBackend>>),
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
    /// Build a language-detection enricher whose detection is `backend`'s
    /// JavaScript callback.
    ///
    /// The callback is `(text: string, candidates: string[]) =>
    /// Promise<LanguageDetection[]>`, where a `LanguageDetection` is
    /// `{ language, confidence, start?, end? }`: a BCP-47 tag, the confidence
    /// in `[0, 1]`, and the byte span it covers when not the whole text.
    /// `candidates` are the languages asserted with
    /// [`Orchestrator::with_languages`](crate::orchestrator::Orchestrator::with_languages)
    /// to choose among, empty for any; with one asserted language the callback
    /// is not called. The resulting [`Enricher`] applies to every stage.
    ///
    /// # Errors
    ///
    /// Rejects a `languageRouted` backend: a detection request has no
    /// language to route by, since finding it is the request's job.
    #[wasm_bindgen(js_name = language)]
    pub fn language(backend: Backend) -> Result<Enricher, ElideError> {
        if backend.routes_by_language() {
            return Err(ElideError::configuration(
                "a language-detection backend cannot be routed by language",
            ));
        }
        Ok(Self(Kind::Language(self::language::build_language(
            backend,
        ))))
    }

    /// Build an OCR enricher whose recognition is `backend`'s JavaScript
    /// callback.
    ///
    /// The callback is `(image: Uint8Array, language?: string) =>
    /// Promise<OcrRegion[]>`, where `language` is the call's BCP-47 tag when
    /// known, and an `OcrRegion` is `{ text, x, y, width, height, confidence? }`
    /// in image-pixel coordinates.
    /// It runs on the browser event loop; the enricher awaits it. The resulting
    /// [`Enricher`] applies only to the image modality.
    #[wasm_bindgen(js_name = ocr)]
    pub fn ocr(backend: Backend) -> Enricher {
        Self(Kind::Ocr(self::ocr::build_ocr(backend)))
    }

    /// Build an STT enricher whose transcription is `backend`'s JavaScript
    /// callback.
    ///
    /// The callback is `(audio: Uint8Array, language?: string) =>
    /// Promise<Segment[]>`, where `language` is the call's BCP-47 tag when
    /// known, and a `Segment` is `{ text, startMs, endMs }`. It runs on the
    /// browser event loop; the enricher awaits it. The resulting [`Enricher`] applies only to
    /// the audio modality.
    #[wasm_bindgen(js_name = stt)]
    pub fn stt(backend: Backend) -> Enricher {
        Self(Kind::Stt(self::stt::build_stt(backend)))
    }
}

impl Enricher {
    /// The error for an enricher offered to a stage that cannot run it.
    fn mismatch(label: &str, modality: &str) -> ElideError {
        ElideError::configuration(format!(
            "{label} enricher does not apply to the {modality} modality"
        ))
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
