//! Per-modality pipeline and its type-erased form, used by the
//! [`Orchestrator`] to drive a document's stream parts across two phases
//! (analyze, then apply).
//!
//! [`ModalityPipeline`] is the concrete typed pipeline (an [`Analyzer`] + an
//! [`Anonymizer`], plus the schema detection a format carries); [`erased`] boxes it behind [`ErasedPipeline`] so the
//! orchestrator can store one per modality and match a stream part to it
//! without naming the modality; [`outcome`] holds the result type the erased
//! analyze method returns; [`metadata`] holds the built-in metadata
//! pipeline's schema detection.
//!
//! [`Orchestrator`]: super::Orchestrator
//! [`ErasedPipeline`]: erased::ErasedPipeline

mod erased;
pub(crate) mod metadata;
mod outcome;

use elide_codec::TypedStream;
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::recognition::Scope;
use elide_core::recognition::annotation::Annotations;
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

pub(crate) use self::erased::ErasedPipeline;
pub(crate) use self::outcome::BoxFuture;

/// The entities a stream carries by its format's schema, before any
/// recognizer runs, kept when the scope's catalog declares them and no
/// exclusion covers them.
pub(crate) type SchemaFn<M> =
    fn(&TypedStream<M>, &Scope, &Annotations<M>) -> Result<Vec<Entity<M>>>;

/// The concrete analyze + redact pipeline for one modality `M`: the
/// [`Analyzer`] and [`Anonymizer`] the erased layer drives against a matched
/// stream part, plus the detection the format's schema carries.
pub(crate) struct ModalityPipeline<M: Modality> {
    pub(crate) analyzer: Analyzer<M>,
    pub(crate) anonymizer: Anonymizer<M>,
    /// Detection the format's schema carries, run beside the analyzer. Set
    /// only for the built-in metadata pipeline; the orchestrator's builders
    /// replace the analyzer and anonymizer and never touch it.
    pub(crate) schema: Option<SchemaFn<M>>,
}

impl<M: Modality> ModalityPipeline<M> {
    /// Pair an analyzer and anonymizer into a pipeline for modality `M`.
    pub(crate) fn new(analyzer: Analyzer<M>, anonymizer: Anonymizer<M>) -> Self {
        Self {
            analyzer,
            anonymizer,
            schema: None,
        }
    }

    /// This pipeline with `schema` detecting beside its analyzer.
    pub(crate) fn with_schema(mut self, schema: SchemaFn<M>) -> Self {
        self.schema = Some(schema);
        self
    }
}
