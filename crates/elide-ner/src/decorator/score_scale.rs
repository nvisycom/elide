//! [`ScoreScale`]: a [`NerBackend`] decorator that scales the score of
//! selected labels.
//!
//! Wraps any inner backend and multiplies the confidence of every emitted
//! span whose label is in a configured set, clamping the result to
//! `[0, 1]`. Use it to demote noisy-but-high-recall labels (multiplier
//! below `1.0`) or to boost labels a model under-scores (above `1.0`):
//!
//! ```ignore
//! let backend = ScoreScale::new(inner)
//!     .with_label(builtins::PERSON_NAME.to_ref())
//!     .with_multiplier(1.2);
//! ```

use std::collections::HashSet;

use async_trait::async_trait;
use elide_core::Result;
use elide_core::entity::LabelRef;
use elide_core::entity::provenance::ModelEvent;

use crate::backend::{NerBackend, NerRequest, NerResponse};

/// Default multiplier: identity. A bare `ScoreScale` leaves scores
/// untouched until a multiplier is set.
const DEFAULT_MULTIPLIER: f32 = 1.0;

/// [`NerBackend`] that scales the confidence of spans whose label is in
/// a configured set.
///
/// Delegates recognition to the wrapped backend, then multiplies the
/// confidence of each matching span by
/// [`multiplier`](Self::with_multiplier), saturating into `[0, 1]`. Spans
/// whose label is not in the set pass through unchanged.
#[derive(Debug, Clone)]
pub struct ScoreScale<B> {
    inner: B,
    labels: HashSet<LabelRef>,
    multiplier: f32,
}

impl<B> ScoreScale<B> {
    /// Wrap `inner`. No labels are scaled and the multiplier starts at the
    /// identity (`1.0`) until configured via
    /// [`with_label`](Self::with_label) /
    /// [`with_labels`](Self::with_labels) and
    /// [`with_multiplier`](Self::with_multiplier).
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            labels: HashSet::new(),
            multiplier: DEFAULT_MULTIPLIER,
        }
    }

    /// Add one label to the set whose scores are scaled.
    #[must_use]
    pub fn with_label(mut self, label: LabelRef) -> Self {
        self.labels.insert(label);
        self
    }

    /// Add several labels to the set whose scores are scaled.
    #[must_use]
    pub fn with_labels(mut self, labels: impl IntoIterator<Item = LabelRef>) -> Self {
        self.labels.extend(labels);
        self
    }

    /// Set the multiplier applied to the configured labels' scores.
    /// Below `1.0` demotes, above `1.0` boosts; the result is clamped to
    /// `[0, 1]`.
    #[must_use]
    pub fn with_multiplier(mut self, multiplier: f32) -> Self {
        self.multiplier = multiplier;
        self
    }

    /// Borrow the wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }
}

#[async_trait]
impl<B: NerBackend> NerBackend for ScoreScale<B> {
    fn provenance(&self) -> ModelEvent {
        self.inner.provenance()
    }

    async fn recognize(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        let mut response = self.inner.recognize(request).await?;
        for span in &mut response.spans {
            if self.labels.contains(&span.label) {
                span.confidence = span.confidence.saturating_mul(self.multiplier);
            }
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::primitive::Confidence;

    use crate::backend::NerSpan;

    use super::*;

    /// Backend that returns a fixed set of spans, ignoring the request.
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

    fn request<'a>(text: &'a str) -> NerRequest<'a> {
        NerRequest {
            text,
            labels: None,
            language: None,
            correlation_id: None,
        }
    }

    #[tokio::test]
    async fn scales_only_configured_labels_and_clamps() {
        let inner = FixedBackend(vec![
            NerSpan::new("EMAIL", 0.5, 0..1),
            NerSpan::new("PHONE", 0.9, 1..2),
        ]);
        // Boost EMAIL by 1.2 (0.5 -> 0.6); PHONE untouched.
        let scaled = ScoreScale::new(inner)
            .with_label(LabelRef::new("EMAIL"))
            .with_multiplier(1.2);

        let out = scaled.recognize(request("x")).await.unwrap();
        assert!(
            (out.spans[0].confidence.get() - 0.6).abs() < 1e-6,
            "{:?}",
            out.spans[0]
        );
        assert_eq!(out.spans[1].confidence, Confidence::clamped(0.9));
    }

    #[tokio::test]
    async fn boost_past_one_clamps_to_one() {
        let inner = FixedBackend(vec![NerSpan::new("EMAIL", 0.9, 0..1)]);
        let scaled = ScoreScale::new(inner)
            .with_labels([LabelRef::new("EMAIL")])
            .with_multiplier(2.0);

        let out = scaled.recognize(request("x")).await.unwrap();
        assert_eq!(out.spans[0].confidence, Confidence::MAX);
    }

    #[tokio::test]
    async fn default_multiplier_is_identity() {
        let inner = FixedBackend(vec![NerSpan::new("EMAIL", 0.42, 0..1)]);
        let scaled = ScoreScale::new(inner).with_label(LabelRef::new("EMAIL"));

        let out = scaled.recognize(request("x")).await.unwrap();
        assert_eq!(out.spans[0].confidence, Confidence::clamped(0.42));
    }
}
