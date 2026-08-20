#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod directives;
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
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

pub use self::directives::Directives;
use self::pipeline::{AnalyzeOutcome, ErasedPipeline, ModalityPipeline};
use self::report::{BodyReport, PartReport};
// `EntityGroup` is the bound on the construction methods; `DocumentSelections`
// is `select`'s return type, and its groups are erased `SelectionGroup`s — all
// named in public signatures, so callers must be able to reach them.
pub use self::report::{DocumentSelections, EntityGroup, Report, SelectionGroup};

/// Drives analyze + redact across a whole document.
///
/// Covers the body and its cross-modality container parts. Built with one
/// [`with_modality`] call per modality the caller wants
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

    /// Set the run-wide default [`Scope`] shared across every modality
    /// pipeline — the caller's analysis-wide assertions (languages,
    /// jurisdictions, tags, catalog, correlation id).
    ///
    /// A `Scope` is modality-free, so one drives the body and every
    /// container part alike; no need to repeat it per [`with_modality`]. A
    /// single analysis can override it with [`Directives::with_scope`], and
    /// supplies its region annotations on the same [`Directives`] passed to
    /// [`analyze`].
    ///
    /// [`with_modality`]: Self::with_modality
    /// [`analyze`]: Self::analyze
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
            }),
        );
        self
    }

    /// Detect the entities of a whole document without redacting: its body
    /// *and* every container part whose modality has a registered pipeline.
    /// Returns an editable [`Report`] to hand to [`anonymize_with`].
    ///
    /// `directives` carries the caller's per-analysis inputs: the region
    /// [`Annotations`] for each modality present in the document, and an
    /// optional [`Scope`] override for this call (falling back to the
    /// orchestrator's run-wide [`with_scope`] default). Pass
    /// [`Directives::new`] for none.
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
    /// [`Annotations`]: elide_core::recognition::annotation::Annotations
    /// [`with_scope`]: Self::with_scope
    /// [`anonymize_with`]: Self::anonymize_with
    /// [`entities`]: Report::entities
    /// [`part_entities`]: Report::part_entities
    pub async fn analyze(
        &self,
        document: &mut UntypedDocumentHandle,
        directives: &Directives,
    ) -> Result<Report> {
        let mut report = Report::new();
        // Per-call scope override wins; else the run-wide default.
        let scope = directives.scope.as_ref().unwrap_or(&self.scope);
        let annotations = &directives.annotations;

        // The body: offer it to each pipeline; the first whose modality
        // matches analyzes it in place. The pipeline's key is the body's
        // modality `TypeId`.
        for (modality, pipeline) in &self.pipelines {
            if let Some(analyzed) = pipeline
                .analyze_in_place(document, scope, annotations)
                .await?
            {
                #[cfg(feature = "usage")]
                let (entities, usage) = analyzed;
                #[cfg(not(feature = "usage"))]
                let entities = analyzed;
                #[cfg(feature = "usage")]
                report.usage.extend(usage);
                report.body = Some(BodyReport {
                    modality: *modality,
                    entities,
                });
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
                match pipeline.analyze(taken, scope, annotations).await? {
                    AnalyzeOutcome::Accepted {
                        modality,
                        handle: retained,
                        entities,
                        #[cfg(feature = "usage")]
                        usage,
                    } => {
                        #[cfg(feature = "usage")]
                        report.usage.extend(usage);
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
    /// Returns the report, now applied: redaction stamps a redaction event
    /// into each entity's provenance, so the returned report's entities carry
    /// the full audit trail (recognition through redaction) — serialize it to
    /// hand the audit to a caller. The report's cached part handles are spent
    /// by applying and are not part of that serialized view.
    ///
    /// [`analyze`]: Self::analyze
    pub async fn anonymize_with(
        &self,
        document: &mut UntypedDocumentHandle,
        mut report: Report,
    ) -> Result<Report> {
        // The body: apply its edited entities in place through the matching
        // pipeline (recovered by the stored modality `TypeId`). Applying
        // mutates the entities — each gains a redaction event — so it happens
        // on `report`'s own groups, which are returned as the audit trail.
        if let Some(body) = report.body.as_mut()
            && let Some(pipeline) = self.pipelines.get(&body.modality)
        {
            pipeline
                .apply_in_place(document, body.entities.as_mut(), &self.scope)
                .await?;
        }

        // The parts: redact each through its cached handle, or re-decode it
        // from the container when the report carries no handle. Collect the
        // redacted bytes first, then splice them back in.
        let mut redactions: Vec<(PartId, Bytes)> = Vec::new();
        for (id, part) in &mut report.parts {
            let Some(pipeline) = self.pipelines.get(&part.modality) else {
                continue; // pipeline for this modality is gone
            };
            let handle = match part.handle.take() {
                Some(handle) => handle,
                // No cached handle (rebuilt/deserialized report): re-decode
                // the part from the container by its id.
                None => {
                    let Some(decoded) = self.redecode_part(document, id).await? else {
                        continue; // part gone, or no codec for it
                    };
                    decoded
                }
            };
            let bytes = pipeline
                .apply_part(handle, part.entities.as_mut(), &self.scope)
                .await?;
            redactions.push((id.clone(), bytes));
        }
        if let Some(c) = document.as_container_mut() {
            for (id, bytes) in redactions {
                c.replace_part(&id, bytes)?;
            }
        }
        Ok(report)
    }

    /// Resolve the operator [`Selection`]s the pipelines would apply to a
    /// whole document's detected entities — the body and every container part
    /// — without reading any document data or redacting.
    ///
    /// The reviewable decision phase surfaced at the document level: it runs
    /// each pipeline's rules over the matching entities in `report` and returns
    /// the picks (which operator hides which entity, and why) as a
    /// [`DocumentSelections`], mirroring the report's own body/parts shape.
    /// Each group is boxed erased so a review layer can inspect it or project
    /// it to serializable [`views`](DocumentSelections::views) before applying.
    /// The body is absent when the report has no body or no pipeline is
    /// registered for its modality; a part is absent when no pipeline covers
    /// its modality.
    ///
    /// Selection reads no data, so this is cheap and side-effect-free — every
    /// part is resolved together at no I/O cost — and the report is left
    /// unchanged. Apply the picks with [`anonymize_with`] (which re-runs
    /// selection internally as part of redacting).
    ///
    /// `scope` is the request [`Scope`] selection runs under — the seam for
    /// per-audience redaction: call this once per audience (a different
    /// `scope.metadata.audience` each time) against the *same* [`Report`] to
    /// produce a different plan per audience from one detection. Pass the
    /// orchestrator's own [scope](Self::with_scope) when there's no per-request
    /// override.
    ///
    /// [`Scope`]: elide_core::recognition::Scope
    /// [`Selection`]: elide_redaction::Selection
    /// [`anonymize_with`]: Self::anonymize_with
    pub fn select(&self, report: &Report, scope: &Scope) -> DocumentSelections {
        let body = report.body.as_ref().and_then(|body| {
            let pipeline = self.pipelines.get(&body.modality)?;
            Some(pipeline.select(body.entities.as_ref(), scope))
        });
        let parts = report
            .parts
            .iter()
            .filter_map(|(id, part)| {
                let pipeline = self.pipelines.get(&part.modality)?;
                Some((id.clone(), pipeline.select(part.entities.as_ref(), scope)))
            })
            .collect();
        DocumentSelections { body, parts }
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
    /// step — redact the whole document in one call. Returns the applied
    /// [`Report`], whose entities carry the full audit trail (recognition
    /// through redaction).
    ///
    /// Use the two phases directly when you need to inspect or edit the
    /// detected entities (drop a false positive, retag) between detection
    /// and redaction.
    ///
    /// [`analyze`]: Self::analyze
    /// [`anonymize_with`]: Self::anonymize_with
    pub async fn anonymize(
        &self,
        document: &mut UntypedDocumentHandle,
        directives: &Directives,
    ) -> Result<Report> {
        let report = self.analyze(document, directives).await?;
        self.anonymize_with(document, report).await
    }
}
