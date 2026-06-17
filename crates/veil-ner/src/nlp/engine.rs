//! [`NlpEngine`]: the producer-side trait that builds the
//! shared-NLP-pass [`TypeMap`] for one or more texts.
//!
//! Engines stamp typed enrichment entries (`LanguageDetections`
//! today; token artifacts when the upstream service supports
//! them) into the returned `TypeMap`. An orchestrator that wants
//! shared NLP runs `process` once per scan, wraps the result in
//! [`Artifacts`], and attaches it to each [`RecognizerInput`] via
//! [`RecognizerInput::with_artifacts`].
//!
//! [`Artifacts`]: veil_core::recognition::Artifacts
//! [`RecognizerInput`]: veil_core::recognition::RecognizerInput
//! [`RecognizerInput::with_artifacts`]: veil_core::recognition::RecognizerInput::with_artifacts
//!
//! Pluggable so different deployment shapes (pure language
//! detection, hosted full-NLP service, future in-process model)
//! can be wired interchangeably. The orchestrator calls `process`
//! (or `process_batch`) once per scan; recognizers and the
//! keyword-boost enhancer borrow the resulting map by reference.

use type_map::concurrent::TypeMap;
use veil_core::Result;
use veil_core::primitive::LanguageTag;

use super::capabilities::NlpCapabilities;

/// Builds the shared-NLP-pass [`TypeMap`] for the orchestrator.
///
/// Engines advertise their capabilities via
/// [`capabilities`] so the orchestrator can refuse impossible
/// compositions at construction time (e.g. wiring a lemma-aware
/// enhancer to an engine that doesn't produce lemmas). Engines also
/// advertise the languages they support so a future per-language
/// registry can route correctly.
///
/// `Send + Sync + 'static`: engines live behind `Arc<dyn _>` in
/// the orchestrator and are shared across recognition tasks.
///
/// [`capabilities`]: Self::capabilities
#[async_trait::async_trait]
pub trait NlpEngine: Send + Sync + 'static {
    /// Languages this engine can produce artifacts for. Empty when
    /// the engine is language-agnostic (e.g. a tokenizer that
    /// works on bytes alone).
    fn supported_languages(&self) -> &[LanguageTag];

    /// What the engine can produce. Advisory; consumers can still
    /// call `process` even when capabilities are off, they just
    /// get an empty map back.
    fn capabilities(&self) -> NlpCapabilities;

    /// Process one text. Returns a [`TypeMap`] populated with one
    /// typed entry per enrichment the engine produced.
    ///
    /// `hint` is the caller-asserted language; engines that can
    /// skip detection when given a hint should do so.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the underlying detection or
    /// inference call fails. Empty input is not an error; engines
    /// should return an empty (or sparsely populated) map.
    async fn process(&self, text: &str, hint: Option<&LanguageTag>) -> Result<TypeMap>;

    /// Process a batch of texts. The default fans out via
    /// [`process`] concurrently; engines with
    /// native batching (`capabilities().batch_native == true`)
    /// should override.
    ///
    /// [`process`]: Self::process
    ///
    /// # Errors
    ///
    /// Returns the first error encountered.
    async fn process_batch(
        &self,
        texts: &[&str],
        hint: Option<&LanguageTag>,
    ) -> Result<Vec<TypeMap>> {
        let mut out = Vec::with_capacity(texts.len());
        for text in texts {
            out.push(self.process(text, hint).await?);
        }
        Ok(out)
    }
}
