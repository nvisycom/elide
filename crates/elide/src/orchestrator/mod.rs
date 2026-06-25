//! [`Orchestrator`]: drive analysis + redaction across a whole document —
//! its own body and the embedded parts of a multi-modal container (a
//! DOCX's images, ahead a PDF's objects).
//!
//! The codec layer exposes a container's parts as opaque byte-blobs (it
//! cannot decode or redact them — it has no recognizers and no registry).
//! The `Orchestrator` is the toolkit-side driver that closes the loop: it
//! holds a [`FormatRegistry`] and one analyze+anonymize pipeline per
//! modality. It detects the document body through its own-modality
//! pipeline and, for each container part, decodes the bytes and detects
//! through the matching pipeline — then applies the (optionally edited)
//! result back.
//!
//! Detection and redaction are two phases, so the entities can be
//! inspected and edited in between:
//!
//! ```ignore
//! let orchestrator = Orchestrator::new(&registry)
//!     .with_modality::<Text>(text_analyzer, text_anonymizer, text_scope)
//!     .with_modality::<Image>(image_analyzer, image_anonymizer, image_scope);
//!
//! let mut report = orchestrator.analyze_document(&mut docx).await?;
//! report.entities::<Text>().unwrap().retain(|e| keep(e)); // drop a false positive
//! orchestrator.apply(&mut docx, report).await?;
//! ```
//!
//! [`anonymize_document`] is the one-call shorthand when no editing is
//! needed. Scope is per-modality, registered alongside each pipeline; a
//! body or part whose modality has no pipeline is left as-is.
//!
//! [`FormatRegistry`]: crate::codec::FormatRegistry
//! [`anonymize_document`]: Orchestrator::anonymize_document

mod pipeline;
mod report;

use std::any::TypeId;
use std::collections::HashMap;

use bytes::Bytes;
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

use self::pipeline::{AnalyzeOutcome, ErasedPipeline, ModalityPipeline};
// `EntityGroup` is re-exported (not just `use`d) because the bound
// `Vec<Entity<M>>: EntityGroup` appears on the public construction methods,
// so callers must be able to name it. Hidden from the docs: it is an
// implementation detail of the report's storage.
#[doc(hidden)]
pub use self::report::EntityGroup;
use self::report::PartReport;
pub use self::report::Report;
use crate::codec::{DocumentHandle, FormatRegistry, PartId};

/// Drives analyze + redact across a document's body and its cross-modality
/// container parts.
///
/// Built with one [`with_modality`] call per modality the caller wants
/// redacted, then run over a document with [`analyze_document`] +
/// [`apply`] (or the [`anonymize_document`] shorthand). Holds the
/// [`FormatRegistry`] used to decode each part and an erased pipeline per
/// modality, keyed by the modality's [`TypeId`] (the same erase-then-
/// re-commit pattern the codec's untyped handle uses).
///
/// [`with_modality`]: Orchestrator::with_modality
/// [`analyze_document`]: Orchestrator::analyze_document
/// [`apply`]: Orchestrator::apply
/// [`anonymize_document`]: Orchestrator::anonymize_document
pub struct Orchestrator<'r> {
    registry: &'r FormatRegistry,
    pipelines: HashMap<TypeId, Box<dyn ErasedPipeline>>,
}

impl<'r> Orchestrator<'r> {
    /// A new orchestrator that decodes parts through `registry`, with no
    /// modality pipelines yet.
    pub fn new(registry: &'r FormatRegistry) -> Self {
        Self {
            registry,
            pipelines: HashMap::new(),
        }
    }

    /// Register the analyze + redact pipeline for modality `M`. A part
    /// that decodes to `M` is driven by this pipeline; parts of a modality
    /// with no registered pipeline pass through untouched. Re-registering
    /// a modality replaces it.
    #[must_use]
    pub fn with_modality<M>(
        mut self,
        analyzer: Analyzer<M>,
        anonymizer: Anonymizer<M>,
        scope: Scope<M>,
    ) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: EntityGroup,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        self.pipelines.insert(
            TypeId::of::<M>(),
            Box::new(ModalityPipeline {
                analyzer,
                anonymizer,
                scope,
            }),
        );
        self
    }

    /// Detect the entities of a whole document without redacting: its
    /// own-modality body *and* every container part whose modality has a
    /// registered pipeline. Returns an editable [`Report`] to hand to
    /// [`apply`].
    ///
    /// The body of `document`'s own modality `M` is analyzed through its
    /// pipeline (skipped when `M` has none). Then, if `document` is a
    /// container, each part is decoded through the registry and analyzed by
    /// the pipeline whose modality matches; the decoded part handle is
    /// retained in the report so [`apply`] can re-drive it. Parts with no
    /// matching pipeline, or that no codec can decode, are omitted.
    ///
    /// Edit the report ([`entities`], [`part_entities`]) before applying.
    ///
    /// [`apply`]: Self::apply
    /// [`entities`]: Report::entities
    /// [`part_entities`]: Report::part_entities
    pub async fn analyze_document<M: Modality>(
        &self,
        document: &mut DocumentHandle<M>,
    ) -> Result<Report>
    where
        Vec<Entity<M>>: EntityGroup,
    {
        let mut document_plan = Report::new();

        // The body: recovered concretely since `M` is known statically.
        if let Some(typed) = self
            .pipelines
            .get(&TypeId::of::<M>())
            .and_then(|p| p.as_any().downcast_ref::<ModalityPipeline<M>>())
        {
            let entities = typed.analyze(document).await?;
            document_plan.body = Some((TypeId::of::<M>(), Box::new(entities)));
        }

        // The parts: decode each, offer it to each pipeline; the matching
        // one analyzes it and the decoded handle is retained for apply.
        if let Some(parts) = document.as_container_mut().map(|c| c.parts()) {
            for part in parts {
                let Ok(handle) = self.registry.decode(part.bytes.clone(), &part.hint).await else {
                    continue; // no codec for this part
                };
                let mut handle = Some(handle);
                for pipeline in self.pipelines.values() {
                    let Some(taken) = handle.take() else { break };
                    match pipeline.analyze_part(taken).await? {
                        AnalyzeOutcome::Accepted {
                            modality,
                            handle: retained,
                            entities,
                        } => {
                            document_plan.parts.insert(
                                part.id.clone(),
                                PartReport {
                                    modality,
                                    handle: retained,
                                    entities,
                                },
                            );
                            break;
                        }
                        AnalyzeOutcome::Rejected(returned) => handle = Some(returned),
                    }
                }
            }
        }

        Ok(document_plan)
    }

    /// Apply a (possibly edited) [`Report`] back onto `document`:
    /// redact the body in place, redact each retained part, and write the
    /// parts back into the container. Re-encode `document` afterward to
    /// serialize the result.
    pub async fn apply<M: Modality>(
        &self,
        document: &mut DocumentHandle<M>,
        plan: Report,
    ) -> Result<()> {
        let Report { body, parts } = plan;

        // The body: apply its edited entities through the `M` pipeline.
        if let Some((type_id, mut entities)) = body
            && type_id == TypeId::of::<M>()
            && let Some(typed) = self
                .pipelines
                .get(&type_id)
                .and_then(|p| p.as_any().downcast_ref::<ModalityPipeline<M>>())
        {
            let entities = entities
                .as_any_mut()
                .downcast_mut::<Vec<Entity<M>>>()
                .expect("plan body modality matches M");
            typed.apply(document, entities).await?;
        }

        // The parts: re-drive each retained handle with its edited
        // entities, then write the redacted bytes back into the container.
        let mut redactions: Vec<(PartId, Bytes)> = Vec::new();
        for (id, mut part) in parts {
            let Some(pipeline) = self.pipelines.get(&part.modality) else {
                continue; // pipeline for this modality is gone
            };
            let bytes = pipeline
                .apply_part(part.handle, part.entities.as_mut())
                .await?;
            redactions.push((id, bytes));
        }
        if let Some(c) = document.as_container_mut() {
            for (id, bytes) in redactions {
                c.replace_part(&id, bytes)?;
            }
        }
        Ok(())
    }

    /// Convenience: [`analyze_document`] then [`apply`] with no editing
    /// step — redact the whole document in one call.
    ///
    /// Use the two phases directly when you need to inspect or edit the
    /// detected entities (drop a false positive, retag) between detection
    /// and redaction.
    ///
    /// [`analyze_document`]: Self::analyze_document
    /// [`apply`]: Self::apply
    pub async fn anonymize_document<M: Modality>(
        &self,
        document: &mut DocumentHandle<M>,
    ) -> Result<()>
    where
        Vec<Entity<M>>: EntityGroup,
    {
        let plan = self.analyze_document(document).await?;
        self.apply(document, plan).await
    }
}
