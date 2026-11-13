//! [`ScoreScaled`]: a NER backend decorator that scales the score of
//! selected labels.
//!
//! Wraps any inner backend and multiplies the confidence of every emitted
//! span whose label is in a configured set, clamping the result to
//! `[0, 1]`. Use it to boost labels a model under-scores (above `1.0`) or to
//! demote noisy-but-high-recall ones (below `1.0`):
//!
//! ```ignore
//! let backend = ScoreScaled::new(inner, [builtins::PERSON_NAME.to_ref()], 0.8);
//! ```

use std::collections::HashSet;

use async_trait::async_trait;
use elide_core::Result;
use elide_core::backend::Backend;
use elide_core::entity::LabelRef;

use crate::backend::{NerRequest, NerResponse};

/// A NER backend that scales the confidence of spans with configured labels.
///
/// Delegates recognition to the wrapped backend, then multiplies the
/// confidence of each matching span by the [`multiplier`], saturating
/// into `[0, 1]`. Spans whose label is not in the set pass through
/// unchanged.
///
/// [`multiplier`]: Self::multiplier
#[derive(Debug, Clone)]
pub struct ScoreScaled<B> {
    inner: B,
    labels: HashSet<LabelRef>,
    multiplier: f32,
}

impl<B> ScoreScaled<B> {
    /// Multiply the scores of `inner`'s spans labelled with any of `labels` by
    /// `multiplier`: below `1.0` demotes, above `1.0` boosts, and the result
    /// is clamped to `[0, 1]`.
    pub fn new(inner: B, labels: impl IntoIterator<Item = LabelRef>, multiplier: f32) -> Self {
        Self {
            inner,
            labels: labels.into_iter().collect(),
            multiplier,
        }
    }

    /// The labels whose scores are scaled.
    pub fn labels(&self) -> &HashSet<LabelRef> {
        &self.labels
    }

    /// The multiplier applied to their scores.
    pub fn multiplier(&self) -> f32 {
        self.multiplier
    }

    /// Borrow the wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }

    /// Scale the confidence of every span whose label is in the configured set.
    fn scale(&self, response: &mut NerResponse) {
        for span in &mut response.spans {
            if self.labels.contains(&span.label) {
                span.confidence = span.confidence.saturating_mul(self.multiplier);
            }
        }
    }
}

#[async_trait]
impl<B> Backend for ScoreScaled<B>
where
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>,
{
    type Request<'a> = NerRequest<'a>;
    type Response = NerResponse;

    async fn call(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        let mut response = self.inner.call(request).await?;
        self.scale(&mut response);
        Ok(response)
    }

    async fn call_batch(&self, requests: Vec<NerRequest<'_>>) -> Result<Vec<NerResponse>> {
        // Forward to the inner backend's batch path so its native batching (if
        // any) is preserved, then scale each response's scores.
        let mut responses = self.inner.call_batch(requests).await?;
        for response in &mut responses {
            self.scale(response);
        }
        Ok(responses)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::ModelId;
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::backend::NerSpan;

    /// Backend that returns a fixed set of spans, ignoring the request.
    struct FixedBackend(Vec<NerSpan>);

    #[async_trait]
    impl Backend for FixedBackend {
        type Request<'a> = NerRequest<'a>;
        type Response = NerResponse;

        async fn call(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
            Ok(NerResponse::new(ModelId::new("fixed"), self.0.clone()))
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
        let scaled = ScoreScaled::new(inner, [LabelRef::new("EMAIL")], 1.2);

        let out = scaled.call(request("x")).await.unwrap();
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
        let scaled = ScoreScaled::new(inner, [LabelRef::new("EMAIL")], 2.0);

        let out = scaled.call(request("x")).await.unwrap();
        assert_eq!(out.spans[0].confidence, Confidence::MAX);
    }
}
