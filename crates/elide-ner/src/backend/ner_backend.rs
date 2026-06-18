//! [`NerBackend`]: the unified per-call NER backend trait.
//!
//! Replaces the previous split between `GlinerBackend` (zero-shot,
//! takes per-call kinds) and `NlpEngine`-produced NER spans
//! (fixed-label, no per-call kinds). The `labels` field on
//! [`NerRequest`] is `Option<&[&str]>`: `Some(...)` for zero-shot
//! backends that take a label allowlist per call, `None` for
//! fixed-label backends whose set of labels is baked into the
//! model.
//!
//! Engines are called from inside [`NerRecognizer::recognize`]; no
//! shared NLP pass, no orchestrator plumbing. Each recognizer holds
//! its own backend and is self-contained.
//!
//! [`NerRecognizer::recognize`]: crate::NerRecognizer

use uuid::Uuid;
use elide_core::Result;
use elide_core::primitive::LanguageTag;
use elide_core::provenance::ModelEvent;

use super::ner_span::RawNerSpan;

/// One per-call NER request handed to a [`NerBackend`].
#[derive(Debug, Clone)]
pub struct NerRequest<'a> {
    /// Source text to scan. Byte offsets in returned spans refer
    /// back into this string.
    pub text: &'a str,
    /// Label names to detect when the backend supports per-call
    /// label selection. `None` means the backend uses its built-in
    /// fixed label set; `Some(slice)` means restrict detection to
    /// the listed names. Empty slice short-circuits the call to no
    /// work in the caller.
    pub labels: Option<&'a [&'a str]>,
    /// Caller-asserted language. Backends that support per-call
    /// language hinting use this; backends that don't ignore it.
    pub language: Option<&'a LanguageTag>,
    /// Correlation UUID for tracing.
    pub correlation_id: Option<Uuid>,
}

/// One per-call NER response from a [`NerBackend`].
///
/// Wraps the raw spans the backend produced. Pre-normalization:
/// labels are still the backend's raw strings; the recognizer
/// applies label-map + ignore-set + low-score demotion before
/// emitting entities.
#[derive(Debug, Clone, Default)]
pub struct NerResponse {
    /// Spans predicted for the request's text, in backend order.
    pub spans: Vec<RawNerSpan>,
}

impl NerResponse {
    /// Construct a response from raw spans.
    #[must_use]
    pub fn new(spans: Vec<RawNerSpan>) -> Self {
        Self { spans }
    }
}

/// Per-call NER backend.
///
/// Implemented by everything that turns `(text, kinds)` into raw
/// NER spans: externalised inference services, local model wrappers
/// (future ORT/Candle backends), and the in-process no-op test stub.
///
/// Object-safe: recognizers hold `Arc<dyn NerBackend>` and dispatch
/// per call.
#[async_trait::async_trait]
pub trait NerBackend: Send + Sync + 'static {
    /// Backend identity (model / service name + provenance detail).
    ///
    /// Distinct from the recognizer's configured name: the
    /// recognizer-level name (e.g. `"company-ner"`) labels the
    /// configured slot, while [`provenance`] identifies the actual
    /// model the backend wraps (e.g. `"noop-ner"`).
    ///
    /// [`provenance`]: Self::provenance
    fn provenance(&self) -> ModelEvent;

    /// Recognise spans for `request`. Returns raw
    /// (pre-normalization) spans; the recognizer applies
    /// label-map + ignore-set + low-score demotion on the way out.
    ///
    /// # Errors
    ///
    /// Returns the underlying transport / parse / inference error.
    async fn recognize(&self, request: NerRequest<'_>) -> Result<NerResponse>;

    /// Batched recognise. Defaults to a sequential fan-out;
    /// backends with native batching should override.
    ///
    /// # Errors
    ///
    /// Returns the first error encountered.
    async fn recognize_batch(&self, requests: &[NerRequest<'_>]) -> Result<Vec<NerResponse>> {
        let mut out = Vec::with_capacity(requests.len());
        for req in requests {
            out.push(self.recognize(req.clone()).await?);
        }
        Ok(out)
    }
}
