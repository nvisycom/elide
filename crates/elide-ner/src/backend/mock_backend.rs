//! [`MockBackend`]: stand-in NER backend for tests, examples, and as a
//! default before a real backend is configured.

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelId;

use super::{NerRequest, NerResponse};

/// Mock NER backend: every call returns an empty response, from the model
/// `mock-ner`.
///
/// Useful as a test stub, in examples that must run without a model, and
/// as the default NER backend when the operator wants the recognizer
/// wired but isn't ready to configure a real backend.
///
/// [`with_dropped_responses`](Self::with_dropped_responses) makes its
/// `call_batch` return fewer responses than requests, so a test can drive a
/// consumer's response-count guard.
#[derive(Debug, Default, Clone, Copy)]
pub struct MockBackend {
    /// How many trailing responses `call_batch` omits; `0` honors the contract.
    dropped: usize,
}

impl MockBackend {
    /// An empty mock backend: every call recognizes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Make `call_batch` return `n` fewer responses than it got requests.
    ///
    /// It violates the one-response-per-request contract, to exercise a
    /// consumer's count guard.
    #[must_use]
    pub fn with_dropped_responses(mut self, n: usize) -> Self {
        self.dropped = n;
        self
    }
}

#[async_trait::async_trait]
impl Backend for MockBackend {
    type Request<'a> = NerRequest<'a>;
    type Response = NerResponse;

    async fn call(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
        Ok(empty())
    }

    async fn call_batch(&self, requests: Vec<NerRequest<'_>>) -> Result<Vec<NerResponse>> {
        let kept = requests.len().saturating_sub(self.dropped);
        Ok((0..kept).map(|_| empty()).collect())
    }
}

/// The mock's answer to any request: no spans.
fn empty() -> NerResponse {
    NerResponse::new(ModelId::new("mock-ner"), Vec::new())
}
