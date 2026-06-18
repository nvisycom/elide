//! [`Enricher<M>`]: the pre-recognition input-enrichment contract.

use std::future::Future;

use crate::error::Error;
use crate::modality::Modality;
use crate::recognition::RecognizerInput;

/// Enriches a [`RecognizerInput`] before recognizers run over it.
///
/// An enricher produces no entities. It fills in per-call context that
/// recognizers consume: detecting the content's language and asserting it
/// onto the input, stamping shared NLP artifacts (tokens, lemmas) keyed by
/// type, and so on. It is the *producer* side of the input; recognizers
/// are the consumers.
///
/// Enrichers run *sequentially*, before the (concurrent) recognition pass,
/// because a later enricher (or a recognizer) may depend on what an
/// earlier one wrote. An analyzer runs its enrichers in order, then hands
/// the enriched input to its recognizers.
pub trait Enricher<M>: Send + Sync
where
    M: Modality,
{
    /// Inspect the input and enrich it in place.
    ///
    /// # Errors
    ///
    /// Returns an error when enrichment fails (e.g. a detection backend is
    /// unreachable). A failed enricher aborts the call before recognition.
    fn enrich(
        &self,
        input: &mut RecognizerInput<M>,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}
