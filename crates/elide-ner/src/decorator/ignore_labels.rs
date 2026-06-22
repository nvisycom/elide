//! [`IgnoreLabels`]: a [`NerBackend`] decorator that drops spans whose
//! label is in a configured set.
//!
//! Wraps any inner backend and removes every span whose label is ignored —
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
use elide_core::entity::LabelRef;
use elide_core::entity::provenance::ModelEvent;

use crate::backend::{NerBackend, NerRequest, NerResponse};

/// [`NerBackend`] that drops spans whose label is in a configured set.
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
}

#[async_trait]
impl<B: NerBackend> NerBackend for IgnoreLabels<B> {
    fn provenance(&self) -> ModelEvent {
        self.inner.provenance()
    }

    async fn recognize(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        let mut response = self.inner.recognize(request).await?;
        response
            .spans
            .retain(|span| !self.labels.contains(&span.label));
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::backend::NerSpan;

    struct FixedBackend(Vec<NerSpan>);

    #[async_trait]
    impl NerBackend for FixedBackend {
        fn provenance(&self) -> ModelEvent {
            ModelEvent {
                name: "fixed".into(),
                ..ModelEvent::default()
            }
        }

        async fn recognize(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
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
        let out = filtered.recognize(request).await.unwrap();
        assert_eq!(out.spans.len(), 1);
        assert_eq!(out.spans[0].label, LabelRef::new("EMAIL"));
        assert_eq!(out.spans[0].confidence, Confidence::clamped(0.9));
    }
}
