//! Per-modality pipeline and its type-erased form, used by the
//! [`Orchestrator`] to drive a document's streams across two phases
//! (analyze, then apply).
//!
//! [`ModalityPipeline`] is the concrete typed pipeline (an [`Analyzer`] + an
//! [`Anonymizer`]); [`erased`] boxes it behind [`ErasedPipeline`] so the
//! orchestrator can store one per modality and match a stream to it
//! without naming the modality, and runs [`flagged`] beside the analyzer to
//! make entities of the hidden content a format labels; [`metadata`] holds the
//! built-in metadata pipeline.
//!
//! [`Orchestrator`]: super::Orchestrator
//! [`ErasedPipeline`]: erased::ErasedPipeline

mod erased;
mod flagged;
pub(crate) mod metadata;

use elide_core::modality::Modality;
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

pub(crate) use self::erased::{BoxFuture, ErasedPipeline, PartAnalysis};

/// The concrete analyze + redact pipeline for one modality `M`: the
/// [`Analyzer`] and [`Anonymizer`] the erased layer drives against a matched
/// stream.
///
/// The anonymizer ends in [`flagged::erase_rule`] where the modality has one,
/// so flagged hidden content no caller rule covers is removed rather than kept.
pub(crate) struct ModalityPipeline<M: Modality> {
    pub(crate) analyzer: Analyzer<M>,
    anonymizer: Anonymizer<M>,
}

impl<M: Modality> ModalityPipeline<M> {
    /// Pair an analyzer and anonymizer into a pipeline for modality `M`.
    pub(crate) fn new(analyzer: Analyzer<M>, anonymizer: Anonymizer<M>) -> Self {
        Self {
            analyzer,
            anonymizer: with_erase_rule(anonymizer),
        }
    }

    /// Replace the anonymizer, keeping the flagged-content rule last.
    pub(crate) fn set_anonymizer(&mut self, anonymizer: Anonymizer<M>) {
        self.anonymizer = with_erase_rule(anonymizer);
    }
}

/// `anonymizer` ending in the modality's flagged-content rule, if it has one.
fn with_erase_rule<M: Modality>(anonymizer: Anonymizer<M>) -> Anonymizer<M> {
    match flagged::erase_rule() {
        Some(rule) => anonymizer.with(rule),
        None => anonymizer,
    }
}
