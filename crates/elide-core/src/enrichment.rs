//! [`Enricher<M>`]: the pre-recognition context-enrichment contract.

use crate::error::Result;
use crate::modality::Modality;
use crate::primitive::ComponentId;
#[cfg(feature = "usage")]
use crate::primitive::ModelUsage;
use crate::recognition::{RecognizerContext, Subject};

/// Enriches a [`RecognizerContext`] before recognizers run over it.
///
/// An enricher produces no entities. It fills in per-call context that
/// recognizers consume: detecting the payload's language and asserting it
/// onto the context, stamping shared NLP artifacts (tokens, lemmas) keyed
/// by type, and so on. It is the *producer* side of the context;
/// recognizers are the consumers.
///
/// Enrichers run *sequentially*, before the (concurrent) recognition pass,
/// because a later enricher (or a recognizer) may depend on what an
/// earlier one wrote. An analyzer runs its enrichers in order, then hands
/// the payload and enriched context to its recognizers.
#[async_trait::async_trait]
pub trait Enricher<M>: Send + Sync
where
    M: Modality,
{
    /// This enricher's identity (name + version), so its usage is labelled
    /// the way a recognizer's is.
    fn id(&self) -> ComponentId;

    /// Inspect the [`Subject`] and enrich it in place, returning any model-usage
    /// detail the enrichment incurred (see [`Enrichment`]). The analysis-wide
    /// [`RecognizerContext`] is read-only (a detector may consult the caller's
    /// asserted languages or correlation id).
    ///
    /// # Errors
    ///
    /// Returns an error when enrichment fails (e.g. a detection backend is
    /// unreachable). A failed enricher aborts the call before recognition.
    ///
    /// [`Subject`]: crate::recognition::Subject
    /// [`RecognizerContext`]: crate::recognition::RecognizerContext
    async fn enrich(
        &self,
        subject: &mut Subject<M>,
        ctx: &RecognizerContext<'_, M>,
    ) -> Result<Enrichment>;
}

/// What an [`Enricher`] returns from one call.
///
/// It produces no entities; under the `usage` feature it carries the
/// `ModelUsage` the enrichment cost, which a model-backed enricher (OCR/STT)
/// attaches with `with_model`. A pure-CPU enricher (language detection) returns
/// [`Enrichment::none`].
#[derive(Debug, Clone, Default)]
pub struct Enrichment {
    /// Model / token detail for a model-backed enricher; `None` otherwise.
    #[cfg(feature = "usage")]
    pub model_usage: Option<ModelUsage>,
}

impl Enrichment {
    /// An enrichment with no model usage, the pure-CPU enricher case.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// An enrichment carrying the [`ModelUsage`] the enrichment cost.
    #[cfg(feature = "usage")]
    #[must_use]
    pub fn with_model(model_usage: ModelUsage) -> Self {
        Self {
            model_usage: Some(model_usage),
        }
    }
}

#[cfg(feature = "usage")]
impl From<ModelUsage> for Enrichment {
    fn from(model_usage: ModelUsage) -> Self {
        Self::with_model(model_usage)
    }
}
