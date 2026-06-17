//! [`BentoBackend`]: NER backend over the externalised
//! `inference-gliner` Bento.
//!
//! Implements [`NerBackend`]. The wire shape mirrors
//! `nvisy_core.ner.v1` from [`nvisycom/inference`]; per-request
//! `correlation_id` propagation rides on the `x-request-id`
//! header. Today the service is zero-shot — it requires a per-call
//! `labels` list — so the backend errors out when called with
//! `labels = None`.
//!
//! Wire compatibility note: the inference service returns the
//! service-side label string verbatim; the recognizer's
//! [`LabelMap`] re-canonicalises it into a workspace label name.
//!
//! [`nvisycom/inference`]: https://github.com/nvisycom/inference
//! [`LabelMap`]: nvisy_core::recognition::LabelMap

use bentoml::prelude::*;
use nvisy_core::entity::ModelProvenance;
use nvisy_core::{Error, Result};
use uuid::Uuid;

use super::bento_types::{WireBatch, WireRequest, WireResponse};
use super::ner_backend::{NerBackend, NerRequest, NerResponse};
use super::ner_span::RawNerSpan;

const COMPONENT: &str = "ner-bento";
const RECOGNIZE_ROUTE: &str = "recognize";

/// Construction parameters for [`BentoBackend`].
#[derive(Debug, Clone)]
pub struct BentoParams {
    /// Base URL of the `inference-gliner` Bento (e.g.
    /// `http://localhost:3000`).
    pub base_url: String,
}

impl BentoParams {
    /// Construct with the given service URL.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }
}

/// [`NerBackend`] backed by an externalised BentoML service.
#[derive(Debug)]
pub struct BentoBackend {
    endpoint: Endpoint,
}

impl BentoBackend {
    /// Build a backend against the given parameters.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying HTTP client cannot be
    /// constructed (invalid `base_url`).
    pub fn new(params: BentoParams) -> Result<Self> {
        let client = Client::builder()
            .with_base_url(&params.base_url)
            .build()
            .map_err(|e| Error::runtime(format!("bentoml client init: {e}"), COMPONENT, false))?;
        Ok(Self {
            endpoint: client.endpoint(RECOGNIZE_ROUTE),
        })
    }
}

#[async_trait::async_trait]
impl NerBackend for BentoBackend {
    fn provenance(&self) -> ModelProvenance {
        ModelProvenance::new("bento-ner")
    }

    #[tracing::instrument(skip_all)]
    async fn recognize(&self, request: NerRequest<'_>) -> Result<NerResponse> {
        let Some(labels) = request.labels else {
            return Err(Error::validation(
                "BentoBackend requires per-call labels (the inference-gliner service is zero-shot)",
                COMPONENT,
            ));
        };
        if labels.is_empty() {
            return Ok(NerResponse::default());
        }

        let language = request.language.map(|l| l.as_str().to_owned());
        let wire_request = WireRequest {
            text: request.text.to_owned(),
            labels: labels.iter().map(|s| (*s).to_owned()).collect(),
            threshold: 0.0,
            language,
        };
        let request_id = request
            .correlation_id
            .unwrap_or_else(Uuid::now_v7)
            .to_string();

        let responses: Vec<WireResponse> = self
            .endpoint
            .clone()
            .with_request_id(&request_id)
            .invoke(&WireBatch {
                requests: vec![wire_request],
            })
            .await
            .map_err(|e| Error::runtime(format!("bento ner call: {e}"), COMPONENT, true))?;

        let mut spans = Vec::new();
        for response in &responses {
            for entity in &response.entities {
                spans.push(RawNerSpan::new(
                    entity.label.clone(),
                    entity.score,
                    entity.start..entity.end,
                ));
            }
        }
        Ok(NerResponse::new(spans))
    }
}
