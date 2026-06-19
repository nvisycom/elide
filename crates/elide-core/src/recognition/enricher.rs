//! [`Enricher<M>`]: the pre-recognition context-enrichment contract.

use std::future::Future;

use crate::error::Result;
use crate::modality::Modality;
use crate::recognition::RecognizerContext;

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
pub trait Enricher<M>: Send + Sync
where
    M: Modality,
{
    /// Inspect `data` and enrich `ctx` in place.
    ///
    /// # Errors
    ///
    /// Returns an error when enrichment fails (e.g. a detection backend is
    /// unreachable). A failed enricher aborts the call before recognition.
    fn enrich(
        &self,
        data: &M::Data,
        ctx: &mut RecognizerContext<'_, M>,
    ) -> impl Future<Output = Result<()>> + Send;
}
