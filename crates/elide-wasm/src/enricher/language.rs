//! The language enricher building block: language detection in JavaScript.
//!
//! The browser build compiles in no language models, which would multiply its
//! size; the page supplies detection instead, from the browser's own
//! `LanguageDetector`, a JS library, or an endpoint.
//! [`Enricher::language`](super::Enricher::language) takes a [`Backend`] whose
//! callback is `(text, candidates) => Promise<LanguageDetection[]>`, and adds
//! each language it detects to the subject for the recognizers that follow.
//!
//! The caller's asserted languages take part as they do for the toolkit's
//! `LinguaEnricher`: one asserted language is authoritative, so detection is
//! skipped; several are the candidates detection chooses among, and a
//! detection outside them is dropped.

use elide::backend::{Backend as BackendTrait, Request, Response};
use elide::enrichment::Enricher;
use elide::entity::audit::ModelId;
use elide::modality::TextRecognizable;
use elide::primitive::{Confidence, LanguageClaim, LanguageTag};
use elide::recognition::{Context, Subject};
use elide::{Component, Error, Result};
use js_sys::Array;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

use crate::backend::{Backend, Composed, JsCallback};

/// The component's name, and the model every response of the callback names.
const NAME: &str = "js-callback-language";

/// One text handed to a language backend, with the languages to choose among.
#[derive(Debug, Clone, Request)]
pub(crate) struct LanguageRequest<'a> {
    /// The text to detect the languages of.
    text: &'a str,
    /// The languages detection chooses among; empty for any.
    candidates: &'a [LanguageTag],
}

/// The languages a language backend detected in its text.
#[derive(Debug, Response)]
pub(crate) struct LanguageResponse {
    /// The model that detected them.
    #[backend(model)]
    model: ModelId,
    /// The detected languages, each over the whole text or a span of it.
    claims: Vec<LanguageClaim>,
}

/// One language the JS callback returns.
#[derive(Deserialize)]
struct JsDetection {
    /// The language, a BCP-47 tag.
    language: String,
    /// The detector's confidence in `[0, 1]`.
    confidence: f32,
    /// Start byte offset of the span it covers; absent for the whole text.
    #[serde(default)]
    start: Option<usize>,
    /// End byte offset (exclusive) of the span it covers.
    #[serde(default)]
    end: Option<usize>,
}

impl JsDetection {
    /// The claim over `text` this detection makes.
    ///
    /// # Errors
    ///
    /// The language is not a BCP-47 tag, or the span is not a range of `text`
    /// on character boundaries: the callback's values are untrusted.
    fn into_claim(self, text: &str) -> Result<LanguageClaim> {
        let language: LanguageTag = self.language.parse().map_err(|e| {
            Error::malformed(format!(
                "the language callback returned an invalid tag `{}`: {e}",
                self.language
            ))
        })?;
        let claim = LanguageClaim::detected(language, Confidence::clamped(self.confidence));
        match (self.start, self.end) {
            (None, None) => Ok(claim),
            (Some(start), Some(end))
                if start <= end
                    && end <= text.len()
                    && text.is_char_boundary(start)
                    && text.is_char_boundary(end) =>
            {
                Ok(claim.with_span(start..end))
            }
            (start, end) => Err(Error::malformed(format!(
                "the language callback returned an invalid span [{start:?}, {end:?}) for text \
                 of {} bytes",
                text.len()
            ))),
        }
    }
}

/// A language backend that defers detection to a JavaScript callback.
pub(crate) struct JsLanguageBackend(JsCallback);

#[async_trait::async_trait]
impl BackendTrait for JsLanguageBackend {
    type Request<'a> = LanguageRequest<'a>;
    type Response = LanguageResponse;

    async fn call(&self, request: LanguageRequest<'_>) -> Result<LanguageResponse> {
        let (text, candidates) = (request.text, request.candidates);
        let detections: Vec<JsDetection> = self
            .0
            .call("language", move || {
                let candidates: Array = candidates
                    .iter()
                    .map(|tag| JsValue::from_str(tag.as_str()))
                    .collect();
                Ok(Array::of2(&JsValue::from_str(text), &candidates))
            })
            .await?;
        let claims = detections
            .into_iter()
            .map(|detection| detection.into_claim(text))
            .collect::<Result<_>>()?;
        Ok(LanguageResponse {
            model: ModelId::new(NAME),
            claims,
        })
    }
}

/// An enricher that detects the languages of a subject's text with a language
/// backend, and adds them to the subject.
#[derive(Component)]
#[component(name = "js-callback-language")]
pub(crate) struct LanguageEnricher<B> {
    backend: B,
}

#[async_trait::async_trait]
impl<M, B> Enricher<M> for LanguageEnricher<B>
where
    M: TextRecognizable,
    B: for<'a> BackendTrait<Request<'a> = LanguageRequest<'a>, Response = LanguageResponse>,
{
    async fn enrich(&self, subject: &mut Subject<M>, ctx: &Context<'_>) -> Result<()> {
        let asserted = ctx.scope().languages();
        if asserted.len() == 1 {
            return Ok(());
        }
        let candidates: Vec<LanguageTag> = asserted
            .iter()
            .map(|claim| claim.language.clone())
            .collect();
        // No recognizable text (an un-transcribed clip, an un-OCR'd image): no
        // language to detect.
        let Some(text) = M::as_text(subject.data(), subject.artifact()) else {
            return Ok(());
        };
        let response = self
            .backend
            .call(LanguageRequest {
                text,
                candidates: &candidates,
            })
            .await?;
        let chosen = |claim: &LanguageClaim| {
            candidates.is_empty()
                || candidates
                    .iter()
                    .any(|tag| tag.primary_subtag() == claim.language.primary_subtag())
        };
        for claim in response.claims.into_iter().filter(chosen) {
            subject.detect_language(claim);
        }
        Ok(())
    }
}

/// Build a language enricher whose detection is `backend`'s JavaScript
/// callback.
pub(super) fn build_language(backend: Backend) -> LanguageEnricher<Composed<JsLanguageBackend>> {
    LanguageEnricher {
        backend: backend.build(&JsLanguageBackend),
    }
}
