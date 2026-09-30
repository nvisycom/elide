//! [`Metered`]: a [`Backend`] decorator that reports each call's usage.

use std::sync::Arc;
use std::time::Instant;

use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;

use super::usage::{ModelUsage, Usage, UsageSink};

/// Wraps a [`Backend`] and reports each call's [`Usage`] to a [`UsageSink`],
/// leaving the response untouched.
///
/// Because it is itself a `Backend` (same request/response, same provenance), it
/// drops in anywhere the inner backend goes; the recognizer or enricher holding
/// it never learns it is metered. It times the call — on both the success and
/// the failure path — reads the response's own units and output count, folds in
/// the model identity from [`provenance`](Backend::provenance), and records the
/// result. This is the one place cost is known, so no other layer carries usage.
pub struct Metered<B> {
    inner: B,
    sink: Arc<dyn UsageSink>,
}

impl<B> Metered<B> {
    /// Meter `inner`, reporting each call to `sink`.
    pub fn new(inner: B, sink: Arc<dyn UsageSink>) -> Self {
        Self { inner, sink }
    }

    /// The wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }
}

#[async_trait::async_trait]
impl<B: Backend> Backend for Metered<B> {
    type Request<'a> = B::Request<'a>;
    type Response = B::Response;

    fn provenance(&self) -> ModelEvent {
        self.inner.provenance()
    }

    async fn call(&self, request: Self::Request<'_>) -> Result<Self::Response> {
        use elide_core::backend::BackendResponse;

        let start = Instant::now();
        let result = self.inner.call(request).await;
        let elapsed = start.elapsed();
        // Record on both paths: a failed call's latency and error kind are signal
        // that would otherwise be lost. Model identity comes from provenance;
        // units and output count from the response, when there is one.
        match &result {
            Ok(response) => {
                let model = ModelUsage::from(self.inner.provenance()).with_units(response.units());
                self.sink
                    .record(Usage::success(elapsed, model, response.output_count()));
            }
            Err(error) => {
                let model = ModelUsage::from(self.inner.provenance());
                self.sink
                    .record(Usage::failure(elapsed, model, error.kind()));
            }
        }
        result
    }
}
