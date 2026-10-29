//! [`LabelScoped`]: a NER backend decorator that scopes a model to the labels
//! it can detect.
//!
//! Wraps any inner backend with the set of labels its model supports. A
//! zero-shot model is asked only for those of the requested labels it can find,
//! rather than the whole catalog, and spans outside the set are dropped:
//!
//! ```ignore
//! let backend = LabelScoped::new(inner, [
//!     builtins::PERSON_NAME.to_ref(),
//!     builtins::ADDRESS.to_ref(),
//! ]);
//! ```

use std::collections::HashSet;

use async_trait::async_trait;
use elide_core::backend::Backend;
use elide_core::entity::audit::ModelEvent;
use elide_core::entity::{Label, LabelRef};
use elide_core::{Error, ErrorKind, Result};

use crate::backend::{NerRequest, NerResponse};

/// A NER backend limited to the labels its model supports.
///
/// The recognizer asks for every label in the call's catalog; which of them a
/// model can detect is a property of the model, so it is declared here, on the
/// backend. On each call this decorator narrows the requested labels to the
/// supported ones, keeping their catalog definitions and order, so a zero-shot
/// model is prompted with only what it can find: fewer labels make it faster
/// and more accurate, and keep the prompt within its input limit. A request
/// left with no supported label is answered with no spans, without calling the
/// model. Spans the model returns outside the set are dropped.
///
/// A request with no labels (an empty catalog, a fixed-label model's path)
/// passes through, and only its spans are filtered.
///
/// Being a backend itself, it composes with the other decorators, and gives
/// each route of a `LanguageRouted` its own label set when the models behind
/// them support different labels.
#[derive(Debug, Clone)]
pub struct LabelScoped<B> {
    inner: B,
    labels: HashSet<LabelRef>,
}

impl<B> LabelScoped<B> {
    /// Limit `inner` to `labels`.
    pub fn new(inner: B, labels: impl IntoIterator<Item = LabelRef>) -> Self {
        Self {
            inner,
            labels: labels.into_iter().collect(),
        }
    }

    /// The labels the model supports.
    pub fn labels(&self) -> &HashSet<LabelRef> {
        &self.labels
    }

    /// Borrow the wrapped backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }

    /// The `requested` labels the model supports, in their requested order.
    fn narrow(&self, requested: &[Label]) -> Vec<Label> {
        requested
            .iter()
            .filter(|label| self.labels.contains(&label.to_ref()))
            .cloned()
            .collect()
    }

    /// Drop every span outside the supported set.
    fn filter(&self, response: &mut NerResponse) {
        response
            .spans
            .retain(|span| self.labels.contains(&span.label));
    }
}

#[async_trait]
impl<B> Backend for LabelScoped<B>
where
    B: for<'a> Backend<Request<'a> = NerRequest<'a>, Response = NerResponse>,
{
    type Request<'a> = NerRequest<'a>;
    type Response = NerResponse;

    fn provenance(&self) -> ModelEvent {
        self.inner.provenance()
    }

    async fn call(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        let narrowed = request.labels.map(|labels| self.narrow(labels));
        if narrowed.as_ref().is_some_and(Vec::is_empty) {
            return Ok(NerResponse::default());
        }
        let request = NerRequest {
            labels: narrowed.as_deref(),
            ..request
        };
        let mut response = self.inner.call(request).await?;
        self.filter(&mut response);
        Ok(response)
    }

    async fn call_batch(&self, requests: Vec<NerRequest<'_>>) -> Result<Vec<NerResponse>> {
        let narrowed: Vec<Option<Vec<Label>>> = requests
            .iter()
            .map(|request| request.labels.map(|labels| self.narrow(labels)))
            .collect();

        // Forward the requests left with a supported label (or with none
        // requested) as one inner batch, keeping its native batching; the rest
        // are answered empty without a model call.
        let mut responses: Vec<NerResponse> =
            requests.iter().map(|_| NerResponse::default()).collect();
        let mut forwarded = Vec::with_capacity(requests.len());
        let mut positions = Vec::with_capacity(requests.len());
        for (position, (request, labels)) in requests.into_iter().zip(&narrowed).enumerate() {
            if labels.as_ref().is_some_and(Vec::is_empty) {
                continue;
            }
            forwarded.push(NerRequest {
                labels: labels.as_deref(),
                ..request
            });
            positions.push(position);
        }
        if forwarded.is_empty() {
            return Ok(responses);
        }

        // The inner backend owes one response per forwarded request. A short
        // batch would leave the tail at its empty default, indistinguishable from
        // "no entities", so it is an error, as it would be without this decorator.
        let forwarded_len = forwarded.len();
        let inner_responses = self.inner.call_batch(forwarded).await?;
        if inner_responses.len() != forwarded_len {
            return Err(Error::new(
                ErrorKind::Provider,
                format!(
                    "NER backend returned {} responses for {} requests",
                    inner_responses.len(),
                    forwarded_len
                ),
            ));
        }
        for (position, mut response) in positions.into_iter().zip(inner_responses) {
            self.filter(&mut response);
            responses[position] = response;
        }
        Ok(responses)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use elide_core::entity::builtins;
    use elide_core::primitive::{Confidence, LanguageTag};

    use super::*;
    use crate::backend::NerSpan;

    /// A backend that records the label names it was asked for and returns one
    /// span per label in `emits`.
    #[derive(Default)]
    struct Recording {
        asked: Mutex<Vec<Option<Vec<String>>>>,
        emits: Vec<LabelRef>,
    }

    #[async_trait]
    impl Backend for Recording {
        type Request<'a> = NerRequest<'a>;
        type Response = NerResponse;

        fn provenance(&self) -> ModelEvent {
            ModelEvent::default()
        }

        async fn call(&self, request: NerRequest<'_>) -> Result<NerResponse> {
            self.asked
                .lock()
                .unwrap()
                .push(request.labels.map(|labels| {
                    labels
                        .iter()
                        .map(|l| l.to_ref().as_str().to_owned())
                        .collect()
                }));
            Ok(NerResponse::new(
                self.emits
                    .iter()
                    .map(|label| NerSpan::with_label(label.clone(), Confidence::MAX, 0..1))
                    .collect(),
            ))
        }
    }

    fn request<'a>(labels: Option<&'a [Label]>) -> NerRequest<'a> {
        NerRequest {
            text: "x",
            labels,
            language: None::<&LanguageTag>,
            correlation_id: None,
        }
    }

    fn catalog_labels() -> Vec<Label> {
        vec![
            (*builtins::EMAIL_ADDRESS).clone(),
            (*builtins::PERSON_NAME).clone(),
            (*builtins::ADDRESS).clone(),
        ]
    }

    fn person_and_address(inner: Recording) -> LabelScoped<Recording> {
        LabelScoped::new(
            inner,
            [builtins::PERSON_NAME.to_ref(), builtins::ADDRESS.to_ref()],
        )
    }

    #[tokio::test]
    async fn asks_only_for_supported_labels_in_requested_order() {
        let backend = person_and_address(Recording::default());
        let labels = catalog_labels();
        backend.call(request(Some(&labels))).await.unwrap();

        let asked = backend.inner().asked.lock().unwrap().clone();
        assert_eq!(
            asked,
            vec![Some(vec!["person_name".to_owned(), "address".to_owned()])]
        );
    }

    #[tokio::test]
    async fn drops_spans_outside_the_supported_set() {
        let inner = Recording {
            emits: vec![
                builtins::PERSON_NAME.to_ref(),
                builtins::EMAIL_ADDRESS.to_ref(),
            ],
            ..Recording::default()
        };
        let backend = person_and_address(inner);
        let labels = catalog_labels();
        let response = backend.call(request(Some(&labels))).await.unwrap();

        let found: Vec<_> = response.spans.iter().map(|s| s.label.clone()).collect();
        assert_eq!(found, vec![builtins::PERSON_NAME.to_ref()]);
    }

    #[tokio::test]
    async fn no_supported_label_requested_skips_the_model() {
        let backend = person_and_address(Recording::default());
        let labels = vec![(*builtins::EMAIL_ADDRESS).clone()];
        let response = backend.call(request(Some(&labels))).await.unwrap();

        assert!(response.spans.is_empty());
        assert!(backend.inner().asked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_requested_labels_passes_through_filtered() {
        let inner = Recording {
            emits: vec![
                builtins::PERSON_NAME.to_ref(),
                builtins::EMAIL_ADDRESS.to_ref(),
            ],
            ..Recording::default()
        };
        let backend = person_and_address(inner);
        let response = backend.call(request(None)).await.unwrap();

        assert_eq!(backend.inner().asked.lock().unwrap().clone(), vec![None]);
        assert_eq!(response.spans.len(), 1);
    }

    #[tokio::test]
    async fn a_batch_skips_unsupported_requests_and_stays_aligned() {
        let inner = Recording {
            emits: vec![builtins::PERSON_NAME.to_ref()],
            ..Recording::default()
        };
        let backend = person_and_address(inner);
        let all = catalog_labels();
        let email_only = vec![(*builtins::EMAIL_ADDRESS).clone()];

        let responses = backend
            .call_batch(vec![
                request(Some(&all)),
                request(Some(&email_only)),
                request(Some(&all)),
            ])
            .await
            .unwrap();

        let counts: Vec<usize> = responses.iter().map(|r| r.spans.len()).collect();
        assert_eq!(counts, vec![1, 0, 1]);
        // The unsupported request never reached the model.
        assert_eq!(backend.inner().asked.lock().unwrap().len(), 2);
    }

    /// A backend that answers every batch with one response too few.
    struct ShortBatch;

    #[async_trait]
    impl Backend for ShortBatch {
        type Request<'a> = NerRequest<'a>;
        type Response = NerResponse;

        fn provenance(&self) -> ModelEvent {
            ModelEvent::default()
        }

        async fn call(&self, _request: NerRequest<'_>) -> Result<NerResponse> {
            Ok(NerResponse::default())
        }

        async fn call_batch(&self, requests: Vec<NerRequest<'_>>) -> Result<Vec<NerResponse>> {
            Ok(requests
                .iter()
                .skip(1)
                .map(|_| NerResponse::default())
                .collect())
        }
    }

    #[tokio::test]
    async fn a_short_inner_batch_is_an_error() {
        let backend = LabelScoped::new(ShortBatch, [builtins::PERSON_NAME.to_ref()]);
        let all = catalog_labels();

        let error = backend
            .call_batch(vec![request(Some(&all)), request(Some(&all))])
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Provider);
    }
}
