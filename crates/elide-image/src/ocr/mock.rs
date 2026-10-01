//! [`MockBackend`]: stand-in OCR backend for tests, examples, and as a
//! default before a real backend is configured.

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;

use super::{OcrRequest, OcrResponse};
use crate::modality::LayoutRegion;

/// Mock OCR backend: returns a fixed set of regions on every call.
///
/// Empty by default ([`new`](Self::new) / [`default`](Default::default)), the
/// no-op stub examples and offline wiring rely on, recognizing nothing. Give it
/// canned regions with [`with`](Self::with) to have every call enrich with the
/// same [`Layout`], for tests that need a real artifact to read back.
///
/// [`Layout`]: crate::modality::Layout
#[derive(Debug, Default, Clone)]
pub struct MockBackend {
    regions: Vec<LayoutRegion>,
    /// How many trailing responses `call_batch` omits; `0` honors the contract.
    dropped: usize,
}

impl MockBackend {
    /// An empty mock backend: every call recognizes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A mock backend that returns `regions` on every call, so the enricher
    /// stamps the same [`Layout`] onto each request.
    ///
    /// [`Layout`]: crate::modality::Layout
    #[must_use]
    pub fn with(regions: Vec<LayoutRegion>) -> Self {
        Self {
            regions,
            dropped: 0,
        }
    }

    /// Make `call_batch` return `n` fewer responses than requests, violating the
    /// one-response-per-request contract, to exercise the enricher's count guard.
    #[must_use]
    pub fn with_dropped_responses(mut self, n: usize) -> Self {
        self.dropped = n;
        self
    }
}

#[async_trait::async_trait]
impl Backend for MockBackend {
    type Request<'a> = OcrRequest<'a>;
    type Response = OcrResponse;

    fn provenance(&self) -> ModelEvent {
        ModelEvent {
            name: "mock-ocr".into(),
            ..ModelEvent::default()
        }
    }

    async fn call(&self, _request: OcrRequest<'_>) -> Result<OcrResponse> {
        Ok(OcrResponse::new(self.regions.clone()))
    }

    async fn call_batch(&self, requests: Vec<OcrRequest<'_>>) -> Result<Vec<OcrResponse>> {
        let kept = requests.len().saturating_sub(self.dropped);
        Ok((0..kept)
            .map(|_| OcrResponse::new(self.regions.clone()))
            .collect())
    }
}
