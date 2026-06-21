//! Backend layer: the [`NerBackend`] trait and its shipped impls.
//!
//! One trait covers zero-shot backends (per-call labels via
//! [`NerRequest::labels`] = `Some(...)`) and fixed-label backends
//! (labels baked into the model, `labels = None`). Backends emit
//! canonical [`NerSpan`]s. Wrap a backend with a [`decorator`] to scale or
//! drop selected labels. The `mock`-gated `MockBackend` (returns no spans;
//! test/example stub) ships here; concrete inference backends live
//! downstream.
//!
//! [`decorator`]: crate::decorator
//! [`NerRecognizer::recognize`]: crate::NerRecognizer
//! [`LabelMap`]: elide_core::recognition::LabelMap

#[cfg(any(test, feature = "mock"))]
mod mock_backend;
mod ner_request;
mod ner_response;

use elide_core::Result;
use elide_core::entity::provenance::ModelEvent;

#[cfg(any(test, feature = "mock"))]
#[cfg_attr(docsrs, doc(cfg(feature = "mock")))]
pub use self::mock_backend::MockBackend;
pub use self::ner_request::NerRequest;
pub use self::ner_response::{NerResponse, NerSpan};

/// Per-call NER backend.
///
/// Implemented by everything that turns `(text, labels)` into canonical
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

    /// Recognise spans for `request`. Returns canonical spans; the
    /// recognizer applies its ignore-set on the way out.
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
