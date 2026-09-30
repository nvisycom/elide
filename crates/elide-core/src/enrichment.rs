//! [`Enricher<M>`]: the pre-recognition context-enrichment contract.

use crate::error::Result;
use crate::modality::Modality;
use crate::primitive::ComponentId;
use crate::recognition::{Context, Subject};

/// Enriches a [`Context`] before recognizers run over it.
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
    /// This enricher's identity (name + version), labelled the way a
    /// recognizer's is.
    fn id(&self) -> ComponentId;

    /// Inspect the [`Subject`] and enrich it in place: an enricher writes its
    /// context (asserted languages, a produced artifact, shared NLP tokens) onto
    /// the subject and produces no return value. The analysis-wide
    /// [`Context`] is read-only (a detector may consult the caller's
    /// asserted languages or correlation id).
    ///
    /// # Errors
    ///
    /// Returns an error when enrichment fails (e.g. a detection backend is
    /// unreachable). A failed enricher aborts the call before recognition.
    ///
    /// [`Subject`]: crate::recognition::Subject
    /// [`Context`]: crate::recognition::Context
    async fn enrich(&self, subject: &mut Subject<M>, ctx: &Context<'_, M>) -> Result<()>;

    /// Enrich a batch of `subjects` in place, under one shared context.
    ///
    /// The default enriches each in turn via [`enrich`](Self::enrich). An enricher
    /// backed by a provider that accepts several inputs in one round-trip (OCR,
    /// speech-to-text) overrides this to build one request per subject and issue a
    /// single [`Backend::call_batch`](crate::backend::Backend::call_batch), turning
    /// N provider round-trips into one. It stays behavior-preserving: each subject
    /// ends enriched exactly as a sequence of [`enrich`](Self::enrich) calls would
    /// leave it, and an already-enriched subject is skipped.
    ///
    /// # Errors
    ///
    /// The first enrichment error; a batched enricher surfaces a whole-batch
    /// failure the same way, aborting the call before recognition.
    async fn enrich_batch(&self, subjects: &mut [Subject<M>], ctx: &Context<'_, M>) -> Result<()> {
        for subject in subjects {
            self.enrich(subject, ctx).await?;
        }
        Ok(())
    }
}
