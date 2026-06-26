#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod pipeline;
mod report;

use std::any::TypeId;
use std::collections::HashMap;

use bytes::Bytes;
use elide_codec::{DocumentHandle, FormatRegistry, Part, PartId, UntypedDocumentHandle};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;
use elide_core::recognition::annotation::Annotations;
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

use self::pipeline::{AnalyzeOutcome, ErasedPipeline, ModalityPipeline};
// `EntityGroup` is re-exported (not just `use`d) because the bound
// `Vec<Entity<M>>: EntityGroup` appears on public methods (`with_modality`,
// `Report::insert_*`), so callers must be able to name it. Hidden from the
// docs: it is an implementation detail of the report's storage.
#[doc(hidden)]
pub use self::report::EntityGroup;
use self::report::PartReport;
pub use self::report::Report;

/// Drives analyze + redact across a document's body and its cross-modality
/// container parts.
///
/// Built with one [`with_modality`] call per modality the caller wants
/// redacted, then run over an [`UntypedDocumentHandle`] with [`analyze`] +
/// [`anonymize_with`] (or the [`anonymize`] shorthand). The document's
/// modality is never named at the call site: the body and every container
/// part are offered to each registered pipeline until one matches, so the
/// orchestrator works the same whatever the document turns out to be.
///
/// Holds the [`FormatRegistry`] used to decode each part and an erased
/// pipeline per modality, keyed by the modality's [`TypeId`].
///
/// [`with_modality`]: Orchestrator::with_modality
/// [`analyze`]: Orchestrator::analyze
/// [`anonymize_with`]: Orchestrator::anonymize_with
/// [`anonymize`]: Orchestrator::anonymize
pub struct Orchestrator<'r> {
    registry: &'r FormatRegistry,
    pipelines: HashMap<TypeId, Box<dyn ErasedPipeline>>,
    scope: Scope,
}

impl<'r> Orchestrator<'r> {
    /// A new orchestrator that decodes parts through `registry`, with no
    /// modality pipelines and an empty [`Scope`].
    pub fn new(registry: &'r FormatRegistry) -> Self {
        Self {
            registry,
            pipelines: HashMap::new(),
            scope: Scope::new(),
        }
    }

    /// Set the [`Scope`] shared across every modality pipeline — the
    /// caller's analysis-wide assertions (languages, jurisdictions, labels,
    /// catalog, correlation id).
    ///
    /// A `Scope` is modality-free, so one drives the body and every
    /// container part alike; no need to repeat it per [`with_modality`].
    /// Per-modality region annotations (inclusions / exclusions) ride on
    /// each pipeline's analyzer instead.
    ///
    /// [`with_modality`]: Self::with_modality
    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// Register the analyze + redact pipeline for modality `M`. A part
    /// that decodes to `M` is driven by this pipeline; parts of a modality
    /// with no registered pipeline pass through untouched. Re-registering
    /// a modality replaces it.
    #[must_use]
    pub fn with_modality<M>(mut self, analyzer: Analyzer<M>, anonymizer: Anonymizer<M>) -> Self
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
                annotations: Annotations::new(),
            }),
        );
        self
    }

    /// Attach the caller's per-request region [`Annotations`] (inclusions /
    /// exclusions) for modality `M`, threaded into that modality's pipeline
    /// at analysis time. A no-op if no pipeline for `M` is registered.
    ///
    /// The modality-free policy is set once with [`with_scope`]; regions are
    /// `M::Location`-typed, so they are registered per modality here.
    ///
    /// [`Annotations`]: elide_core::recognition::annotation::Annotations
    /// [`with_scope`]: Self::with_scope
    #[must_use]
    pub fn with_annotations<M: Modality>(mut self, annotations: Annotations<M>) -> Self {
        if let Some(pipeline) = self.pipelines.get_mut(&TypeId::of::<M>()) {
            pipeline.set_annotations(Box::new(annotations));
        }
        self
    }

    /// Detect the entities of a whole document without redacting: its body
    /// *and* every container part whose modality has a registered pipeline.
    /// Returns an editable [`Report`] to hand to [`anonymize_with`].
    ///
    /// The body is offered to each pipeline until one matches its modality;
    /// that pipeline analyzes it. Then, if `document` is a container, each
    /// part is decoded through the registry and matched the same way, its
    /// decoded handle retained in the report as a same-process cache for
    /// apply. The body, and any part, with no matching pipeline (or that no
    /// codec can decode) is omitted.
    ///
    /// Edit the report ([`entities`], [`part_entities`]) before applying.
    ///
    /// [`anonymize_with`]: Self::anonymize_with
    /// [`entities`]: Report::entities
    /// [`part_entities`]: Report::part_entities
    pub async fn analyze(&self, document: &mut UntypedDocumentHandle) -> Result<Report> {
        let mut report = Report::new();

        // The body: offer it to each pipeline; the first whose modality
        // matches analyzes it in place. The pipeline's key is the body's
        // modality `TypeId`.
        for (modality, pipeline) in &self.pipelines {
            if let Some(entities) = pipeline.analyze_in_place(document, &self.scope).await? {
                report.body = Some((*modality, entities));
                break;
            }
        }

        // The parts: decode each, offer it to each pipeline; the matching
        // one analyzes it and its handle is cached for the apply phase.
        let parts = document.as_container_mut().map(|c| c.parts());
        for part in parts.into_iter().flatten() {
            let Ok(handle) = self.registry.decode(part.bytes.clone(), &part.hint).await else {
                continue; // no codec for this part
            };
            let mut handle = Some(handle);
            for pipeline in self.pipelines.values() {
                let Some(taken) = handle.take() else { break };
                match pipeline.analyze(taken, &self.scope).await? {
                    AnalyzeOutcome::Accepted {
                        modality,
                        handle: retained,
                        entities,
                    } => {
                        report.parts.insert(
                            part.id.clone(),
                            PartReport {
                                modality,
                                handle: Some(retained),
                                entities,
                            },
                        );
                        break;
                    }
                    AnalyzeOutcome::Rejected(returned) => handle = Some(returned),
                }
            }
        }

        Ok(report)
    }

    /// Apply a (possibly edited) [`Report`] back onto `document`: redact the
    /// body in place and redact each container part, writing the parts back
    /// into the container. Re-encode `document` afterward to serialize the
    /// result.
    ///
    /// Each part is redacted through its cached handle when the report still
    /// carries one (the same-process path from [`analyze`]); for a report
    /// built by hand or rebuilt from serialized entities, the part is
    /// re-decoded from `document`'s container by its id. So `document` must
    /// be the same document the report describes.
    ///
    /// [`analyze`]: Self::analyze
    pub async fn anonymize_with(
        &self,
        document: &mut UntypedDocumentHandle,
        report: Report,
    ) -> Result<()> {
        let Report { body, parts } = report;

        // The body: apply its edited entities in place through the matching
        // pipeline (recovered by the stored modality `TypeId`).
        if let Some((modality, mut entities)) = body
            && let Some(pipeline) = self.pipelines.get(&modality)
        {
            pipeline.apply_in_place(document, entities.as_mut()).await?;
        }

        // The parts: redact each through its cached handle, or re-decode it
        // from the container when the report carries no handle. Collect the
        // redacted bytes first, then splice them back in.
        let mut redactions: Vec<(PartId, Bytes)> = Vec::new();
        for (id, mut part) in parts {
            let Some(pipeline) = self.pipelines.get(&part.modality) else {
                continue; // pipeline for this modality is gone
            };
            let handle = match part.handle.take() {
                Some(handle) => handle,
                // No cached handle (rebuilt/deserialized report): re-decode
                // the part from the container by its id.
                None => {
                    let Some(decoded) = self.redecode_part(document, &id).await? else {
                        continue; // part gone, or no codec for it
                    };
                    decoded
                }
            };
            let bytes = pipeline.apply_part(handle, part.entities.as_mut()).await?;
            redactions.push((id, bytes));
        }
        if let Some(c) = document.as_container_mut() {
            for (id, bytes) in redactions {
                c.replace_part(&id, bytes)?;
            }
        }
        Ok(())
    }

    /// Re-decode the container part `id` from `document` into a handle, or
    /// `None` if the document is not a container, has no such part, or no
    /// codec can decode it. The apply-time fallback when a report carries no
    /// cached handle.
    async fn redecode_part(
        &self,
        document: &mut UntypedDocumentHandle,
        id: &PartId,
    ) -> Result<Option<UntypedDocumentHandle>> {
        let Some(part) = document
            .as_container_mut()
            .map(|c| c.parts())
            .into_iter()
            .flatten()
            .find(|p: &Part| &p.id == id)
        else {
            return Ok(None);
        };
        Ok(self.registry.decode(part.bytes, &part.hint).await.ok())
    }

    /// Convenience: [`analyze`] then [`anonymize_with`] with no editing
    /// step — redact the whole document in one call.
    ///
    /// Use the two phases directly when you need to inspect or edit the
    /// detected entities (drop a false positive, retag) between detection
    /// and redaction.
    ///
    /// [`analyze`]: Self::analyze
    /// [`anonymize_with`]: Self::anonymize_with
    pub async fn anonymize(&self, document: &mut UntypedDocumentHandle) -> Result<()> {
        let report = self.analyze(document).await?;
        self.anonymize_with(document, report).await
    }
}
