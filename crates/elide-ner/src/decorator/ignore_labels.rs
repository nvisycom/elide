//! [`IgnoreLabels`]: a NER backend decorator that drops spans whose
//! label is in a configured set.
//!
//! Wraps any inner backend and removes every span whose label is ignored,
//! for filtering out labels a model emits but the caller doesn't care
//! about (`O` from BIO tagging, `MISC` from generic schemas, …):
//!
//! ```ignore
//! let backend = IgnoreLabels::new(inner)
//!     .with_label(LabelRef::new("MISC"));
//! ```

use std::collections::HashSet;

use async_trait::async_trait;
use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::LabelRef;
use elide_core::entity::audit::ModelEvent;

use crate::backend::{NerRequest, NerResponse};

/// A NER backend that drops spans whose label is in a configured set.
///
/// Delegates recognition to the wrapped backend, then removes every span
/// whose label is ignored. Spans whose label is not in the set pass
/// through unchanged.
#[derive(Debug, Clone)]
pub struct IgnoreLabels<B> {
    inner: B,
    labels: HashSet<LabelRef>,
}

impl<B> IgnoreLabels<B> {
    /// Wrap `inner`. No labels are ignored until configured via
    /// [`with_label`] / [`with_labels`].
    ///
    /// [`with_label`]: Self::with_label
    /// [`with_labels`]: Self::with_labels
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            labels: HashSet::new(),
        }
    }

    /// Add one label to the ignore set.
    #[must_use]
    pub fn with_label(mut self, label: LabelRef) -> Self {
        self.labels.insert(label);
        self
    }

    /// Add several labels to the ignore set.
    #[must_use]
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = LabelRef>) -> Self {
        self.labels.extend(labels);
        self
    }

    /// Borrow the wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }

    /// Drop every span whose label is in the ignore set.
    fn filter(&self, response: &mut NerResponse) {
        response
            .spans
            .retain(|span| !self.labels.contains(&span.label));
    }
}

#[async_trait]
impl<B> Backend for IgnoreLabels<B>
where
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>,
{
    type Request<'a> = NerRequest<'a>;
    type Response = NerResponse;

    fn provenance(&self) -> ModelEvent {
        self.inner.provenance()
    }

    async fn call(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        let mut response = self.inner.call(request).await?;
        self.filter(&mut response);
        Ok(response)
    }

    async fn call_batch(&self, requests: Vec<NerRequest<'_>>) -> Result<Vec<NerResponse>> {
        // Forward to the inner backend's batch path so its native batching (if
        // any) is preserved, then apply the label filter to each response.
        let mut responses = self.inner.call_batch(requests).await?;
        for response in &mut responses {
            self.filter(response);
        }
        Ok(responses)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::backend::NerSpan;

    struct FixedBackend(Vec<NerSpan>);

    #[async_trait]
    impl Backend for FixedBackend {
        type Request<'a> = NerRequest<'a>;
        type Response = NerResponse;

        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "fixed".into(),
                ..ModelEvent::default()
            }
        }

        async fn call(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
            Ok(NerResponse::new(self.0.clone()))
        }
    }

    #[tokio::test]
    async fn drops_ignored_labels_keeps_the_rest() {
        let inner = FixedBackend(vec![
            NerSpan::new("MISC", 0.9, 0..1),
            NerSpan::new("EMAIL", 0.9, 1..2),
        ]);
        let filtered = IgnoreLabels::new(inner).with_label(LabelRef::new("MISC"));

        let request = NerRequest {
            text: "x",
            labels: None,
            language: None,
            correlation_id: None,
        };
        let out = filtered.call(request).await.unwrap();
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].label, LabelRef::new("EMAIL"));
        assert_eq!(out.spans[0].confidence, Confidence::clamped(0.9));
    }

    /// A backend that records how it was called, so a test can prove the
    /// decorator forwarded the batch rather than fanning out per request.
    #[derive(Default)]
    struct CountingBackend {
        batch_calls: std::sync::atomic::AtomicUsize,
        single_calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl Backend for CountingBackend {
        type Request<'a> = NerRequest<'a>;
        type Response = NerResponse;

        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "counting".into(),
                ..ModelEvent::default()
            }
        }

        async fn call(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
            self.single_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(NerResponse::new(vec![NerSpan::new("MISC", 0.9, 0..1)]))
        }

        async fn call_batch(&self, requests: Vec<NerRequest<'_>>) -> Result<Vec<NerResponse>> {
            self.batch_calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(requests
                .iter()
                .map(|_| NerResponse::new(vec![NerSpan::new("MISC", 0.9, 0..1)]))
                .collect())
        }
    }

    fn request() -> NerRequest<'static> {
        NerRequest {
            text: "x",
            labels: None,
            language: None,
            correlation_id: None,
        }
    }

    #[tokio::test]
    async fn call_batch_forwards_to_inner_and_filters_each() {
        use std::sync::atomic::Ordering;

        let filtered =
            IgnoreLabels::new(CountingBackend::default()).with_label(LabelRef::new("MISC"));
        let out = filtered
            .call_batch(vec![request(), request()])
            .await
            .unwrap();

        // The decorator used the inner's batch path once, not two single calls.
        assert_eq!(filtered.inner().batch_calls.load(Ordering::Relaxed), 1);
        assert_eq!(filtered.inner().single_calls.load(Ordering::Relaxed), 0);
        // And filtered every response: MISC is dropped, so each is empty.
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|r| r.spans.is_empty()));
    }
}
