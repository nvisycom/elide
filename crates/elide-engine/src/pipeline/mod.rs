//! Per-modality pipeline and its type-erased form, used by the
//! [`Orchestrator`] to drive a document's stream parts across two phases
//! (analyze, then apply).
//!
//! [`ModalityPipeline`] is the concrete typed pipeline (an [`Analyzer`] + an
//! [`Anonymizer`]); [`erased`] boxes it behind [`ErasedPipeline`] so the
//! orchestrator can store one per modality and match a stream part to it
//! without naming the modality; [`outcome`] holds the result type the erased
//! analyze method returns.
//!
//! [`Orchestrator`]: super::Orchestrator
//! [`ErasedPipeline`]: erased::ErasedPipeline

mod erased;
mod outcome;

use elide_core::modality::Modality;
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

pub(crate) use self::erased::ErasedPipeline;
pub(crate) use self::outcome::BoxFuture;

/// The concrete analyze + redact pipeline for one modality `M`: the
/// [`Analyzer`] and [`Anonymizer`] the erased layer drives against a matched
/// stream part.
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
