//! Per-modality pipeline and its type-erased form, used by the
//! [`Orchestrator`] to drive a document's body and its container parts across
//! two phases (analyze, then apply).
//!
//! [`ModalityPipeline`] is the concrete typed pipeline (an [`Analyzer`] + an
//! [`Anonymizer`]); [`erased`] boxes it behind [`ErasedPipeline`] so the
//! orchestrator can store one per modality and match a part to it without naming
//! the modality; [`outcome`] holds the result types the erased methods return.
//!
//! [`Orchestrator`]: super::Orchestrator
//! [`ErasedPipeline`]: erased::ErasedPipeline

mod erased;
mod outcome;

use elide_codec::DocumentHandle;
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;
use elide_core::recognition::annotation::Annotations;
use elide_detection::{Analysis, Analyzer};
use elide_redaction::Anonymizer;

pub(crate) use self::erased::ErasedPipeline;
pub(crate) use self::outcome::{AnalyzeOutcome, BoxFuture};

/// The concrete analyze + redact pipeline for one modality `M`.
///
/// The [`Scope`] and region [`Annotations`] are supplied per analysis (via
/// [`Directives`]) as arguments to [`analyze`].
///
/// [`Annotations`]: elide_core::recognition::annotation::Annotations
/// [`Directives`]: super::Directives
/// [`analyze`]: Self::analyze
pub(crate) struct ModalityPipeline<M: Modality> {
    pub(crate) analyzer: Analyzer<M>,
    pub(crate) anonymizer: Anonymizer<M>,
}

impl<M: Modality> ModalityPipeline<M> {
    /// Pair an analyzer and anonymizer into a pipeline for modality `M`.
    pub(crate) fn new(analyzer: Analyzer<M>, anonymizer: Anonymizer<M>) -> Self {
        Self {
            analyzer,
            anonymizer,
        }
    }
}

impl<M> ModalityPipeline<M>
where
    M: Modality,
    DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
{
    /// Detect the entities in `handle` (in source coordinates), without
    /// redacting. `artifact` is the prior enrichment to restore, `Some` (even
    /// when empty) has recognition run against it instead of re-invoking the
    /// model; `None` is a first pass, so the enricher runs and produces the
    /// artifact itself. The caller may edit the returned set before applying.
    pub(super) async fn analyze(
        &self,
        handle: &mut DocumentHandle<M>,
        scope: &Scope,
        annotations: &Annotations<M>,
        artifact: Option<M::Artifact>,
    ) -> Result<Analysis<M>> {
        self.analyzer
            .analyze_stream_in(handle, scope, annotations, artifact)
            .await
    }

    /// Apply `entities` to `handle` in place: the redactions land in the
    /// handle, ready for its eventual `encode`. `scope` is passed to selection
    /// so scope-aware rules can branch on request context.
    pub(super) async fn apply(
        &self,
        handle: &mut DocumentHandle<M>,
        entities: &mut [Entity<M>],
        scope: &Scope,
    ) -> Result<()> {
        self.anonymizer.anonymize(handle, entities, scope).await
    }
}
