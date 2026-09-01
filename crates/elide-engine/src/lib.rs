#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analysis;
mod directives;
mod pipeline;

use std::any::TypeId;
use std::collections::HashMap;

use bytes::Bytes;
use elide_codec::{DocumentHandle, FormatRegistry, Part, PartId, UntypedDocumentHandle};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, NoArtifact, StreamDataReader};
use elide_core::recognition::Scope;
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

pub use self::analysis::{AnalyzedDocument, ArtifactSet, Report, ReportDeserializer};
// `EntityGroup` / `ArtifactGroup` are bounds on the construction methods — named
// in public signatures, so callers must be able to reach them.
pub use self::analysis::{ArtifactGroup, EntityGroup};
use self::analysis::{BodyReport, ModalityRegistry, PartReport};
pub use self::directives::Directives;
use self::pipeline::{AnalyzeOutcome, ErasedPipeline, ModalityPipeline};

/// Drives analyze + redact across a whole document.
///
/// Covers the body and its cross-modality container parts. Built with one
/// [`with_modality`] call per modality the caller wants redacted, plus a
/// [`with_registry`] for the codec that decodes container parts (a body-only
/// document needs none). Then run over an [`UntypedDocumentHandle`] with
/// [`analyze`] + [`anonymize_with`] (or the [`anonymize`] shorthand). The
/// document's modality is never named at the call site: the body and every
/// container part are offered to each registered pipeline until one matches, so
/// the orchestrator works the same whatever the document turns out to be.
///
/// Holds the [`FormatRegistry`] used to decode each container part and an
/// erased pipeline per modality, keyed by the modality's [`TypeId`].
///
/// [`with_modality`]: Orchestrator::with_modality
/// [`with_registry`]: Orchestrator::with_registry
/// [`analyze`]: Orchestrator::analyze
/// [`anonymize_with`]: Orchestrator::anonymize_with
/// [`anonymize`]: Orchestrator::anonymize
#[derive(Default)]
pub struct Orchestrator {
    registry: FormatRegistry,
    pipelines: HashMap<TypeId, Box<dyn ErasedPipeline>>,
    /// Per-modality parsers for reconstructing a serialized [`Report`], keyed by
    /// modality name. Populated alongside `pipelines` in [`with_modality`], so a
    /// deserialized report is rebuilt against exactly the registered modalities.
    ///
    /// [`with_modality`]: Self::with_modality
    groups: ModalityRegistry,
    scope: Scope,
}

impl Orchestrator {
    /// A new orchestrator: no modality pipelines, an empty [`Scope`], and an
    /// empty [`FormatRegistry`] (so container parts do not decode until one is
    /// supplied via [`with_registry`]).
    ///
    /// [`with_registry`]: Self::with_registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the [`FormatRegistry`] used to decode a document's container parts,
    /// taking ownership. A body-only document needs no registry; a container
    /// (DOCX, …) needs one that covers its part formats — typically
    /// [`FormatRegistry::with_builtin`].
    ///
    /// [`FormatRegistry::with_builtin`]: elide_codec::FormatRegistry::with_builtin
    #[must_use]
    pub fn with_registry(mut self, registry: FormatRegistry) -> Self {
        self.registry = registry;
        self
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
    /// a modality replaces both halves.
    ///
    /// When a modality only ever detects *or* only ever redacts, register just
    /// the half it needs with [`with_analyzer`] / [`with_anonymizer`] instead —
    /// the other half defaults to a no-op.
    ///
    /// [`with_analyzer`]: Self::with_analyzer
    /// [`with_anonymizer`]: Self::with_anonymizer
    #[must_use]
    pub fn with_modality<M>(mut self, analyzer: Analyzer<M>, anonymizer: Anonymizer<M>) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: EntityGroup + serde::de::DeserializeOwned,
        M::Artifact: crate::analysis::ArtifactGroup + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        self.pipelines.insert(
            TypeId::of::<M>(),
            Box::new(ModalityPipeline {
                analyzer,
                anonymizer,
            }),
        );
        // Register the parser that reconstructs this modality's group from the
        // wire, so `deserialize_report` can route it back by name.
        self.groups.register::<M>();
        self
    }

    /// Register (or update) only the *analyzer* for modality `M` — the detection
    /// half. If a pipeline for `M` already exists, its analyzer is replaced and
    /// its anonymizer kept; otherwise a new pipeline is created with a no-op
    /// [`Anonymizer::new`] as the redaction half.
    ///
    /// For a modality a caller only ever [`analyze`]s (never redacts): there is
    /// no need to fabricate an anonymizer to satisfy [`with_modality`].
    ///
    /// [`analyze`]: Self::analyze
    /// [`with_modality`]: Self::with_modality
    #[must_use]
    pub fn with_analyzer<M>(mut self, analyzer: Analyzer<M>) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: EntityGroup + serde::de::DeserializeOwned,
        M::Artifact: crate::analysis::ArtifactGroup + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        match self.modality_pipeline_mut::<M>() {
            Some(pipeline) => pipeline.analyzer = analyzer,
            None => self.insert_pipeline::<M>(analyzer, Anonymizer::new()),
        }
        self
    }

    /// Register (or update) only the *anonymizer* for modality `M` — the
    /// redaction half. If a pipeline for `M` already exists, its anonymizer is
    /// replaced and its analyzer kept; otherwise a new pipeline is created with a
    /// no-op [`Analyzer::new`] as the detection half.
    ///
    /// For a modality whose entities are supplied out-of-band (a rebuilt report)
    /// and only redacted: there is no need to fabricate an analyzer.
    ///
    /// [`with_modality`]: Self::with_modality
    #[must_use]
    pub fn with_anonymizer<M>(mut self, anonymizer: Anonymizer<M>) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: EntityGroup + serde::de::DeserializeOwned,
        M::Artifact: crate::analysis::ArtifactGroup + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        match self.modality_pipeline_mut::<M>() {
            Some(pipeline) => pipeline.anonymizer = anonymizer,
            None => self.insert_pipeline::<M>(Analyzer::new(), anonymizer),
        }
        self
    }

    /// The registered pipeline for `M`, recovered to its concrete type, or
    /// `None` if none is registered. The downcast holds because the pipeline is
    /// keyed by `TypeId::of::<M>()`.
    fn modality_pipeline_mut<M>(&mut self) -> Option<&mut ModalityPipeline<M>>
    where
        M: Modality,
    {
        self.pipelines
            .get_mut(&TypeId::of::<M>())?
            .as_any_mut()
            .downcast_mut::<ModalityPipeline<M>>()
    }

    /// Insert a fresh pipeline for `M` and register its deserialize parser.
    fn insert_pipeline<M>(&mut self, analyzer: Analyzer<M>, anonymizer: Anonymizer<M>)
    where
        M: Modality,
        Vec<Entity<M>>: EntityGroup + serde::de::DeserializeOwned,
        M::Artifact: crate::analysis::ArtifactGroup + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        self.pipelines.insert(
            TypeId::of::<M>(),
            Box::new(ModalityPipeline {
                analyzer,
                anonymizer,
            }),
        );
        self.groups.register::<M>();
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
    ) -> Result<AnalyzedDocument> {
        let mut report = Report::new();
        let mut artifacts = ArtifactSet::new();
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
                let (entities, artifact, usage) = analyzed;
                #[cfg(not(feature = "usage"))]
                let (entities, artifact) = analyzed;
                #[cfg(feature = "usage")]
                report.usage.extend(usage);
                let name = entities.modality_name();
                report.body = Some(BodyReport {
                    modality: *modality,
                    entities,
                });
                artifacts.set_body(*modality, name, artifact);
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
                        artifact,
                        #[cfg(feature = "usage")]
                        usage,
                    } => {
                        #[cfg(feature = "usage")]
                        report.usage.extend(usage);
                        let name = entities.modality_name();
                        report.parts.insert(
                            part.id.clone(),
                            PartReport {
                                modality,
                                handle: Some(retained),
                                entities,
                            },
                        );
                        artifacts.set_part(part.id.clone(), modality, name, artifact);
                        break;
                    }
                    AnalyzeOutcome::Rejected(returned) => handle = Some(returned),
                }
            }
        }

        Ok(AnalyzedDocument { report, artifacts })
    }

    /// Re-detect over `document`, seeding each group's recognition with the
    /// enrichment artifact from `prior` so the OCR/transcript is reused rather
    /// than recomputed. The re-run counterpart to [`analyze`](Self::analyze),
    /// for detection separated in time from a first pass: after the review gap,
    /// re-run recognition (e.g. under a narrowed `scope` to add one recognizer)
    /// without paying for OCR/STT again.
    ///
    /// A group `prior` has no artifact for — a body/part it never analyzed —
    /// re-runs from an empty (default) artifact, i.e. it re-enriches. `document`
    /// must be the same document the `prior` artifacts were produced from.
    pub async fn re_analyze(
        &self,
        document: &mut UntypedDocumentHandle,
        prior: &ArtifactSet,
        directives: &Directives,
    ) -> Result<AnalyzedDocument> {
        let mut report = Report::new();
        let mut artifacts = ArtifactSet::new();
        let scope = directives.scope.as_ref().unwrap_or(&self.scope);
        let annotations = &directives.annotations;
        let empty: Box<dyn ArtifactGroup> = Box::new(NoArtifact);

        // The body, re-analyzed against its prior artifact.
        for (modality, pipeline) in &self.pipelines {
            let seed = prior
                .body
                .as_ref()
                .filter(|b| b.modality == *modality)
                .map_or(empty.as_ref(), |b| b.artifact.as_ref());
            if let Some(analyzed) = pipeline
                .re_analyze_in_place(document, scope, annotations, seed)
                .await?
            {
                #[cfg(feature = "usage")]
                let (entities, artifact, usage) = analyzed;
                #[cfg(not(feature = "usage"))]
                let (entities, artifact) = analyzed;
                #[cfg(feature = "usage")]
                report.usage.extend(usage);
                let name = entities.modality_name();
                report.body = Some(BodyReport {
                    modality: *modality,
                    entities,
                });
                artifacts.set_body(*modality, name, artifact);
                break;
            }
        }

        // Each part, re-analyzed against its prior artifact.
        let parts = document.as_container_mut().map(|c| c.parts());
        for part in parts.into_iter().flatten() {
            let Ok(handle) = self.registry.decode(part.bytes.clone(), &part.hint).await else {
                continue; // no codec for this part
            };
            let prior_part = prior.parts.get(&part.id);
            let mut handle = Some(handle);
            for pipeline in self.pipelines.values() {
                let Some(taken) = handle.take() else { break };
                // Seed with the prior part's artifact; the erased method
                // downcasts against `M`, so a mismatched artifact resolves to the
                // default (empty) and the part re-enriches.
                let seed = prior_part.map_or(empty.as_ref(), |p| p.artifact.as_ref());
                match pipeline.re_analyze(taken, scope, annotations, seed).await? {
                    AnalyzeOutcome::Accepted {
                        modality,
                        handle: retained,
                        entities,
                        artifact,
                        #[cfg(feature = "usage")]
                        usage,
                    } => {
                        #[cfg(feature = "usage")]
                        report.usage.extend(usage);
                        let name = entities.modality_name();
                        report.parts.insert(
                            part.id.clone(),
                            PartReport {
                                modality,
                                handle: Some(retained),
                                entities,
                            },
                        );
                        artifacts.set_part(part.id.clone(), modality, name, artifact);
                        break;
                    }
                    AnalyzeOutcome::Rejected(returned) => handle = Some(returned),
                }
            }
        }

        Ok(AnalyzedDocument { report, artifacts })
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
        let analyzed = self.analyze(document, directives).await?;
        self.anonymize_with(document, analyzed.report).await
    }

    /// Reconstruct a [`Report`] from its serialized wire form, routing each
    /// group back to the modality that produced it.
    ///
    /// The serialized report tags each group with its modality name but not the
    /// concrete type, and deserialization is not object-safe — so the report
    /// cannot rebuild itself. This orchestrator can: [`with_modality`] registered
    /// a parser per modality, so each group is parsed as the right
    /// `Vec<Entity<M>>`. Reconstructed parts carry no cached handle;
    /// [`anonymize_with`] re-decodes them from the container, exactly as for any
    /// report built by hand.
    ///
    /// The round trip for a review layer: [`analyze`], serialize the report, ship
    /// it out for editing, then `deserialize_report` it back here and
    /// [`anonymize_with`]. Both ends configure the same modalities.
    ///
    /// A group naming a modality this orchestrator has no pipeline for is handled
    /// by what it would cost to drop it, deliberately splitting the difference
    /// with [`analyze`] (which silently ignores a part whose modality has no
    /// pipeline): an *empty* such group is skipped — the part could not have been
    /// redacted anyway, so nothing is lost, and the round trip succeeds just as a
    /// fresh analysis of the same document would; a *non-empty* one is a hard
    /// error, since its entities may carry a reviewer's edits that silently
    /// dropping the group would lose.
    ///
    /// # Errors
    ///
    /// Returns a [`MalformedInput`] error if the payload is not a valid report,
    /// or if a group carries entities under a modality this orchestrator has no
    /// pipeline for (see above).
    ///
    /// [`with_modality`]: Self::with_modality
    /// [`analyze`]: Self::analyze
    /// [`anonymize_with`]: Self::anonymize_with
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub fn deserialize_report<'de, D>(&self, deserializer: D) -> Result<Report>
    where
        D: serde::Deserializer<'de>,
    {
        self.groups.deserialize(deserializer)
    }
}
