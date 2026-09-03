#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analysis;
mod directives;
mod part_id;
mod pipeline;

use std::any::TypeId;
use std::collections::{HashMap, VecDeque};
use std::path::Path;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{DocumentHandle, FormatRegistry, LocalId, Part, UntypedDocumentHandle};
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, NoArtifact, StreamDataReader};
use elide_core::recognition::Scope;
use elide_core::{Error, ErrorKind, Result};
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

pub use self::analysis::{AnalyzedDocument, ArtifactSet, Report, ReportDeserializer};
// `EntityGroup` / `ArtifactGroup` are the crate-internal erased storage the
// report and artifact set hold; the public construction bounds are expressed in
// terms of `serde::Serialize`, which the blanket impls satisfy, so neither trait
// is named in any public signature and both stay `pub(crate)`.
use self::analysis::{ArtifactGroup, ModalityRegistry, PartReport};
use self::directives::AnnotationSet;
pub use self::directives::Directives;
pub use self::part_id::PartId;
use self::pipeline::{AnalyzeOutcome, BoxFuture, ErasedPipeline, ModalityPipeline};

/// How deep the orchestrator descends into nested containers before erroring.
///
/// A container part that is itself a container is recursed into so its own parts
/// are redacted; a document is at most a handful of levels deep in practice (a
/// bundle → a DOCX → an embedded spreadsheet → its media is depth 4). The bound
/// exists only to stop an adversarial or self-referential archive, a zip that
/// contains itself, from recursing without end; exceeding it is a hard error,
/// not a silent stop, so nothing nested is left un-redacted.
const MAX_CONTAINER_DEPTH: usize = 8;

/// Drives analyze + redact across a set of documents.
///
/// Covers each [`Document`]'s own content and its cross-modality container
/// parts. Built with one [`with_modality`] call per modality the caller wants
/// redacted, plus a [`with_registry`] for the codec that decodes container
/// parts. Then run over a slice of [`Document`]s with [`analyze`] +
/// [`anonymize_with`] (or the [`anonymize`] shorthand); a single document is a
/// one-element slice. The document's modality is never named at the call site:
/// each document and every container part are offered to each registered
/// pipeline until one matches, so the orchestrator works the same whatever a
/// document turns out to be.
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
    /// (DOCX, …) needs one that covers its part formats, typically
    /// [`FormatRegistry::with_builtin`].
    ///
    /// [`FormatRegistry::with_builtin`]: elide_codec::FormatRegistry::with_builtin
    #[must_use]
    pub fn with_registry(mut self, registry: FormatRegistry) -> Self {
        self.registry = registry;
        self
    }

    /// Set the run-wide default [`Scope`] shared across every modality
    /// pipeline, the caller's analysis-wide assertions (languages,
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
    /// the half it needs with [`with_analyzer`] / [`with_anonymizer`] instead,
    /// the other half defaults to a no-op.
    ///
    /// [`with_analyzer`]: Self::with_analyzer
    /// [`with_anonymizer`]: Self::with_anonymizer
    #[must_use]
    pub fn with_modality<M>(mut self, analyzer: Analyzer<M>, anonymizer: Anonymizer<M>) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        self.pipelines.insert(
            TypeId::of::<M>(),
            Box::new(ModalityPipeline::new(analyzer, anonymizer)),
        );
        // Register the parser that reconstructs this modality's group from the
        // wire, so `deserialize_report` can route it back by name.
        self.groups.register::<M>();
        self
    }

    /// Register (or update) only the *analyzer* for modality `M`, the detection
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
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        match self.modality_pipeline_mut::<M>() {
            Some(pipeline) => pipeline.analyzer = analyzer,
            None => self.insert_pipeline::<M>(analyzer, Anonymizer::new()),
        }
        self
    }

    /// Register (or update) only the *anonymizer* for modality `M`, the
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
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
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
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
        DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
    {
        self.pipelines.insert(
            TypeId::of::<M>(),
            Box::new(ModalityPipeline::new(analyzer, anonymizer)),
        );
        self.groups.register::<M>();
    }

    /// Detect the entities of a set of documents without redacting: each
    /// [`Document`]'s own content *and* every container part whose modality has
    /// a registered pipeline. A single document is a one-element slice; a scan
    /// stack or a batch shipped together is redacted as one logical unit, so
    /// entities are found (and later removed) consistently across all of them.
    /// Returns an [`AnalyzedDocument`]: its editable [`report`] to hand to
    /// [`anonymize_with`], and the [`artifacts`] (the OCR/transcript enrichment)
    /// to persist across a review gap and pass to [`re_analyze`], which reuses
    /// them instead of re-enriching.
    ///
    /// `directives` applies to the whole set: the region [`Annotations`] for
    /// each modality present, and an optional [`Scope`] override for this call
    /// (falling back to the orchestrator's run-wide [`with_scope`] default).
    /// Pass [`Directives::new`] for none.
    ///
    /// Every document is a top-level file: its own content is analyzed and keyed
    /// under its [`name`](Document::name) as a depth-1 part, and its container
    /// parts are flattened beneath that name, so two files sharing a local part
    /// id (two scans, each `page-1.png`) never collide. Each document, and each
    /// part, is offered to every pipeline until one matches its modality; that
    /// pipeline analyzes it. A document or part with no matching pipeline (or
    /// that no codec can decode) passes through untouched.
    ///
    /// Read a part with [`part_entities`] keyed by its name; edit the report
    /// ([`part_entities`], [`part_entities_mut`]) before applying. For the
    /// common single-document case, [`entities`] reads the sole document's own
    /// entities directly.
    ///
    /// [`Annotations`]: elide_core::recognition::annotation::Annotations
    /// [`with_scope`]: Self::with_scope
    /// [`anonymize_with`]: Self::anonymize_with
    /// [`re_analyze`]: Self::re_analyze
    /// [`report`]: AnalyzedDocument::report
    /// [`artifacts`]: AnalyzedDocument::artifacts
    /// [`entities`]: Report::entities
    /// [`part_entities`]: Report::part_entities
    /// [`part_entities_mut`]: Report::part_entities_mut
    pub async fn analyze(
        &self,
        mut documents: impl AsDocuments,
        directives: &Directives,
    ) -> Result<AnalyzedDocument> {
        // A first pass is a re-run seeded with nothing: every document and part
        // starts from the empty (default) artifact, so it enriches from scratch.
        // Sharing the driver keeps the two entry points from drifting.
        self.drive(
            documents.as_documents_mut(),
            &ArtifactSet::new(),
            directives,
        )
        .await
    }

    /// Re-detect over `documents`, seeding each group's recognition with the
    /// enrichment artifact from `prior` so the OCR/transcript is reused rather
    /// than recomputed. The re-run counterpart to [`analyze`](Self::analyze),
    /// for detection separated in time from a first pass: after the review gap,
    /// re-run recognition (e.g. under a narrowed `scope` to add one recognizer)
    /// without paying for OCR/STT again.
    ///
    /// A group `prior` has no artifact for, a document or part it never
    /// analyzed, re-runs from an empty (default) artifact, i.e. it re-enriches.
    /// `documents` must be the same set the `prior` artifacts were produced from.
    pub async fn re_analyze(
        &self,
        mut documents: impl AsDocuments,
        prior: &ArtifactSet,
        directives: &Directives,
    ) -> Result<AnalyzedDocument> {
        self.drive(documents.as_documents_mut(), prior, directives)
            .await
    }

    /// The shared analyze driver behind [`analyze`](Self::analyze) and
    /// [`re_analyze`](Self::re_analyze): drive each document's own content and
    /// every container part through the matching pipeline, each seeded with its
    /// prior enrichment from `prior` (an empty set on a first pass, so every
    /// group enriches from scratch). The seeded artifact is downcast against the
    /// group's modality by the erased pipeline, so a `prior` entry for a
    /// different modality resolves to the default (empty) and that group
    /// re-enriches, a document's own content and its parts resolve their seed
    /// the same way.
    async fn drive(
        &self,
        documents: &mut [Document],
        prior: &ArtifactSet,
        directives: &Directives,
    ) -> Result<AnalyzedDocument> {
        let mut report = Report::new();
        let mut artifacts = ArtifactSet::new();
        // Per-call scope override wins; else the run-wide default.
        let scope = directives.scope.as_ref().unwrap_or(&self.scope);
        let annotations = &directives.annotations;
        let empty: Box<dyn ArtifactGroup> = Box::new(NoArtifact);

        // A document's name is its depth-1 `PartId`, the key its own content and
        // every nested part hang under. Two documents sharing a name would key to
        // the same path, so the second would overwrite the first in the report and
        // never be reached at apply. Reject the collision up front rather than
        // silently drop a document's redaction.
        let mut names = std::collections::HashSet::new();
        for document in documents.iter() {
            if !names.insert(document.name.as_str()) {
                return Err(Error::new(
                    ErrorKind::MalformedInput,
                    format!("duplicate document name `{}` in the set", document.name),
                ));
            }
        }

        // Each document's own content: analyzed in place, stored as a depth-1
        // part keyed by the document's name. This is where a leaf document (a
        // plain image) is reached; a container document's own content (a DOCX's
        // body text) is reached here too, and its parts by the flatten below.
        // Seeded from the document's prior artifact so a re-run reuses it.
        for document in documents.iter_mut() {
            let id = PartId::leaf(document.name.clone());
            let seed = prior
                .parts
                .get(&id)
                .map_or(empty.as_ref(), |e| e.artifact.as_ref());
            self.analyze_in_place_into(
                &mut document.handle,
                id,
                seed,
                scope,
                annotations,
                &mut report,
                &mut artifacts,
            )
            .await?;
        }

        // Every document's container parts, keyed under the document's name so
        // no two documents' parts collide. Re-borrows the set now that per-
        // document analysis is done.
        self.flatten(
            documents,
            prior,
            scope,
            annotations,
            &mut report,
            &mut artifacts,
        )
        .await?;

        Ok(AnalyzedDocument { report, artifacts })
    }

    /// Analyze a *borrowed* handle's own content in place, one document's
    /// content in the set, offering it to each pipeline until one matches its
    /// modality, seeded with `seed` (its prior enrichment). The findings and any
    /// enrichment artifact are stored as the part keyed by `id` (the document's
    /// name, its depth-1 path). A handle whose modality no pipeline covers stores
    /// nothing (an intentional pass-through), exactly as an undecodable container
    /// part does.
    ///
    /// The part stored here carries no cached handle: the handle is borrowed (not
    /// owned), so apply redacts it *in place* through that same borrow rather
    /// than re-decoding.
    #[allow(clippy::too_many_arguments)]
    async fn analyze_in_place_into(
        &self,
        handle: &mut UntypedDocumentHandle,
        id: PartId,
        seed: &dyn ArtifactGroup,
        scope: &Scope,
        annotations: &AnnotationSet,
        report: &mut Report,
        artifacts: &mut ArtifactSet,
    ) -> Result<()> {
        for (modality, pipeline) in &self.pipelines {
            let Some(analyzed) = pipeline
                .analyze_in_place(handle, scope, annotations, seed)
                .await?
            else {
                continue; // not this pipeline's modality
            };
            #[cfg(feature = "usage")]
            let (entities, artifact, usage) = analyzed;
            #[cfg(not(feature = "usage"))]
            let (entities, artifact) = analyzed;
            #[cfg(feature = "usage")]
            report.usage.extend(usage);
            let name = entities.modality_name();
            report.parts.insert(
                id.clone(),
                PartReport {
                    modality: *modality,
                    // Borrowed handle: applied in place, never cached.
                    handle: None,
                    entities,
                },
            );
            if let Some(artifact) = artifact {
                artifacts.set_part(id.clone(), *modality, name, artifact);
            }
            break;
        }
        Ok(())
    }

    /// Flatten the document's container tree into `report` / `artifacts`,
    /// walking it **iteratively** (a work queue, not recursion, so an
    /// adversarially deep archive can never blow the stack) breadth-first from
    /// the top container down. A part that is itself a container is enqueued and
    /// its own parts reached in turn (an image in a DOCX embedded in a bundle),
    /// rather than silently passed through; a leaf is driven through a pipeline.
    ///
    /// A part that no codec can decode is opaque, not a container, so it is left
    /// as-is, the same pass-through as a leaf whose modality has no pipeline.
    /// A part that *does* decode to a container is descended into; past
    /// [`MAX_CONTAINER_DEPTH`] that is an error, never a silent drop, so a
    /// redaction tool can never quietly leave a nested document un-redacted.
    ///
    /// `documents` seed the queue: each is a top whose parts are keyed under the
    /// document's own name (prefix [`PartId::leaf`] of the name), so two
    /// documents sharing a local part id stay distinct paths. Each enqueued item
    /// is `(owned container handle, its tree path, its depth)`, the only state
    /// that varies per container; the shared `prior` / `scope` / `report` /
    /// `artifacts` stay in scope rather than being threaded down.
    async fn flatten(
        &self,
        documents: &mut [Document],
        prior: &ArtifactSet,
        scope: &Scope,
        annotations: &AnnotationSet,
        report: &mut Report,
        artifacts: &mut ArtifactSet,
    ) -> Result<()> {
        // Each container to walk: its parts, its tree path, its depth. A top
        // (root) document is borrowed and handled first (its parts seed the
        // queue); every deeper container is an *owned* decoded handle.
        enum Container<'a> {
            Top(&'a mut UntypedDocumentHandle),
            Nested(UntypedDocumentHandle),
        }
        let mut queue: VecDeque<(Container<'_>, PartId, usize)> = VecDeque::new();
        // Each document is a top whose parts are keyed under its own name, so two
        // documents sharing a local part id stay distinct paths.
        for document in documents {
            let prefix = PartId::leaf(document.name.clone());
            queue.push_back((Container::Top(&mut document.handle), prefix, 0));
        }

        while let Some((mut container, prefix, depth)) = queue.pop_front() {
            let handle = match &mut container {
                Container::Top(h) => &mut **h,
                Container::Nested(h) => h,
            };
            let Some(parts) = handle.as_container_mut().map(|c| c.parts()) else {
                continue; // not a container (a leaf pushed here would be a bug)
            };

            for part in parts {
                let part_id = prefix.child(part.id);
                let Ok(mut child) = self.registry.decode(part.bytes.clone(), &part.hint).await
                else {
                    continue; // no codec for this part, opaque, left as-is
                };
                let is_container = child.as_container_mut().is_some();

                // Past the depth bound a nested container is a hard error, never
                // a silent drop; check before analyzing so nothing deeper runs.
                if is_container && depth + 1 > MAX_CONTAINER_DEPTH {
                    return Err(elide_core::Error::new(
                        elide_core::ErrorKind::MalformedInput,
                        format!(
                            "container nesting exceeds the depth limit of \
                             {MAX_CONTAINER_DEPTH} at part `{part_id}`"
                        ),
                    ));
                }

                // Analyze this part's *own* content, a leaf's payload, or a
                // nested container's body (a DOCX's text), keyed under its path.
                // A part whose modality has no pipeline passes through untouched.
                let walk = self
                    .analyze_part(
                        &part_id,
                        child,
                        prior,
                        scope,
                        annotations,
                        report,
                        artifacts,
                    )
                    .await?;

                // A container is then enqueued so its own parts are reached too ,
                // both its body (analyzed above) and its parts get redacted.
                if let Some(handle) = walk
                    && is_container
                {
                    queue.push_back((Container::Nested(handle), part_id, depth + 1));
                }
            }
        }
        Ok(())
    }

    /// Analyze one part's own content: offer `handle` to each pipeline until one
    /// matches its modality, storing the detected entities (and any enrichment
    /// artifact) under `part_id`. Used for a leaf *and* for a nested container's
    /// body (a DOCX's text), a container is both analyzed here for its body and
    /// then walked for its parts. A part whose modality has no pipeline is left
    /// untouched (an intentional pass-through).
    ///
    /// Returns the live handle so the caller can walk further into it (a
    /// container's parts), or [`None`] when no pipeline accepted the part. The
    /// handle is never cached in the report, every part is re-decoded by path at
    /// apply, so it always comes back live when a pipeline matched.
    #[allow(clippy::too_many_arguments)]
    async fn analyze_part(
        &self,
        part_id: &PartId,
        handle: UntypedDocumentHandle,
        prior: &ArtifactSet,
        scope: &Scope,
        annotations: &AnnotationSet,
        report: &mut Report,
        artifacts: &mut ArtifactSet,
    ) -> Result<Option<UntypedDocumentHandle>> {
        let empty: Box<dyn ArtifactGroup> = Box::new(NoArtifact);
        let prior_part = prior.parts.get(part_id);
        let mut handle = Some(handle);
        for pipeline in self.pipelines.values() {
            let Some(taken) = handle.take() else { break };
            let seed = prior_part.map_or(empty.as_ref(), |p| p.artifact.as_ref());
            match pipeline.analyze(taken, scope, annotations, seed).await? {
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
                        part_id.clone(),
                        PartReport {
                            modality,
                            // Re-decoded by path at apply, never cached.
                            handle: None,
                            entities,
                        },
                    );
                    if let Some(artifact) = artifact {
                        artifacts.set_part(part_id.clone(), modality, name, artifact);
                    }
                    return Ok(Some(retained));
                }
                AnalyzeOutcome::Rejected(returned) => handle = Some(returned),
            }
        }
        // No pipeline matched; return the handle unchanged so a container still
        // has its parts walked.
        Ok(handle)
    }

    /// Apply a (possibly edited) [`Report`] back onto the same `documents`:
    /// redact each document's own content in place and each nested part, folding
    /// the redacted bytes back up each document's container tree. Re-encode each
    /// document afterward (`document.handle.encode()`) to serialize the result.
    ///
    /// A document's own content is a depth-1 part, redacted in place through the
    /// document's borrowed handle (it was borrowed at analysis, so nothing was
    /// cached). A nested part is redacted through its cached handle when the
    /// report still carries one (the same-process path from [`analyze`]), else
    /// re-decoded by its path from its document (a rebuilt or deserialized
    /// report), then folded bottom-up. `documents` must be the same set the
    /// report describes.
    ///
    /// Returns the report, now applied: redaction stamps a redaction event
    /// into each entity's provenance, so the returned report's entities carry
    /// the full audit trail (recognition through redaction), serialize it to
    /// hand the audit to a caller. The report's cached part handles are spent
    /// by applying and are not part of that serialized view.
    ///
    /// [`analyze`]: Self::analyze
    pub async fn anonymize_with(
        &self,
        mut documents: impl AsDocuments,
        mut report: Report,
    ) -> Result<Report> {
        let documents = documents.as_documents_mut();
        // A document's own content is a depth-1 part whose one segment is the
        // document's name; a nested part is deeper. Apply the own-content parts
        // in place (through each document's borrowed handle), and collect the
        // nested parts' redacted bytes to fold up afterward, a single bottom-up
        // pass, each byte encoded once.
        //
        // Applying mutates the entities (each gains a redaction event), so it
        // happens on `report`'s own groups, which are returned as the audit trail.
        let mut redactions: Vec<(PartId, Bytes)> = Vec::new();
        for (id, part) in &mut report.parts {
            let Some(pipeline) = self.pipelines.get(&part.modality) else {
                continue; // pipeline for this modality is gone
            };

            // Depth-1: a document's own content. Redact it in place through the
            // document's borrowed handle.
            if id.depth() == 1 {
                let name = id.last_segment();
                if let Some(document) = documents.iter_mut().find(|d| d.name == name) {
                    pipeline
                        .apply_in_place(&mut document.handle, part.entities.as_mut(), &self.scope)
                        .await?;
                }
                continue;
            }

            // Deeper: a nested part. Use its cached handle (the same-process fast
            // path) or re-decode it by its path, redact to bytes, and stage it
            // for the bottom-up fold.
            let handle = match part.handle.take() {
                Some(handle) => handle,
                None => {
                    let Some(decoded) = self.decode_by_path(documents, id).await? else {
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
        self.fold_redactions(documents, redactions).await?;
        Ok(report)
    }

    /// Fold each redaction, a part's redacted bytes, keyed by its tree path,
    /// back into its document, bottom-up. A part at path `[…parent, leaf]` writes
    /// its bytes into the container at `[…parent]` under local id `leaf`; once a
    /// nested container has all its child replacements it is re-decoded, its
    /// replacements applied, and re-encoded, producing bytes for *its* parent,
    /// a redaction one level shallower. Repeatedly resolving the deepest parent
    /// first cascades this to the top, where the depth-1 parents write into each
    /// document's own container, which re-encodes itself.
    ///
    /// A nested container that carries *its own* redaction (a DOCX whose body
    /// text was redacted) as well as descendant parts is folded onto its own
    /// redacted bytes: those bytes, staged as a replacement in its parent, are
    /// the base its descendant replacements apply to, so the container's own
    /// redaction is preserved rather than lost to a re-decode of the original.
    fn fold_redactions<'a>(
        &'a self,
        documents: &'a mut [Document],
        redactions: Vec<(PartId, Bytes)>,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // Group replacements by parent path (all segments but the last). The
            // parent that names a *top* container, a one-segment document path ,
            // writes straight into that document's handle; a deeper parent
            // re-encodes and folds into its own parent.
            let mut by_parent: HashMap<PartId, Vec<(LocalId, Bytes)>> = HashMap::new();
            for (id, bytes) in redactions {
                if let Some((parent, local)) = id.split_last() {
                    by_parent.entry(parent).or_default().push((local, bytes));
                }
            }

            // Resolve one parent per step, deepest first, so a nested container
            // re-encodes before the level above needs its bytes. A resolved
            // nested container enqueues its own bytes for *its* parent.
            while let Some(parent) = deepest_parent(&by_parent) {
                let children = by_parent.remove(&parent).unwrap_or_default();
                if let Some(top) = top_container(documents, &parent) {
                    // A top container (a document's own handle): write straight
                    // into it; it re-encodes itself at the end.
                    if let Some(c) = top.as_container_mut() {
                        for (local, bytes) in children {
                            c.replace_part(&local, bytes)?;
                        }
                    }
                    continue;
                }

                // A nested container. If its *own* redaction was staged (as a
                // replacement of `parent.local` in its grandparent), fold onto
                // those bytes, the container's own redaction, and consume that
                // staged entry so it is not also applied verbatim (which would
                // clobber the fold). Otherwise decode the container as-is from the
                // original document.
                let own_bytes = parent
                    .split_last()
                    .and_then(|(gp, local)| take_staged(&mut by_parent, &gp, &local));
                let decoded = match &own_bytes {
                    // Decode the container's own redacted bytes, resolving the
                    // format from its local id's extension (as the flatten did).
                    Some(bytes) => {
                        let hint = parent
                            .last_segment_id()
                            .and_then(LocalId::extension)
                            .unwrap_or("");
                        self.registry.decode(bytes.clone(), hint).await.ok()
                    }
                    None => self.decode_by_path(documents, &parent).await?,
                };
                let Some(mut handle) = decoded else {
                    continue; // the container is gone or undecodable
                };
                if let Some(c) = handle.as_container_mut() {
                    for (local, bytes) in children {
                        c.replace_part(&local, bytes)?;
                    }
                }
                let bytes = handle.encode()?.to_bytes();
                if let Some((grandparent, local)) = parent.split_last() {
                    by_parent
                        .entry(grandparent)
                        .or_default()
                        .push((local, bytes));
                }
            }
            Ok(())
        })
    }

    /// Decode the part at tree path `id` by walking the container tree of its
    /// `documents` one segment at a time, the last segment names the target
    /// part, the earlier ones the containers to descend through. `None` when any
    /// segment names no part, or no codec can decode a step. The apply-time and
    /// nested path for a part whose live handle wasn't cached at analysis.
    ///
    /// The path's *first* segment selects the document; the rest walk within it.
    async fn decode_by_path(
        &self,
        documents: &mut [Document],
        id: &PartId,
    ) -> Result<Option<UntypedDocumentHandle>> {
        let segments: Vec<&str> = id.segments().collect();
        // Resolve the document and the segments still to walk within it. The
        // leading segment selects the document; the document itself (no remaining
        // segments) has no part to decode here.
        let Some((root, rest)) = root_container(documents, &segments) else {
            return Ok(None);
        };
        let Some((last, prefix)) = rest.split_last() else {
            return Ok(None);
        };

        // Descend through the prefix containers to the target part's immediate
        // container, decoding each step from the previous.
        let mut current: Option<UntypedDocumentHandle> = None;
        for seg in prefix {
            let container = current.as_mut().map_or_else(
                || root.as_container_mut(),
                UntypedDocumentHandle::as_container_mut,
            );
            let Some(part) = container
                .map(|c| c.parts())
                .into_iter()
                .flatten()
                .find(|p: &Part| p.id == **seg)
            else {
                return Ok(None);
            };
            let Ok(handle) = self.registry.decode(part.bytes, &part.hint).await else {
                return Ok(None);
            };
            current = Some(handle);
        }

        // Decode the target part from its immediate container.
        let container = current.as_mut().map_or_else(
            || root.as_container_mut(),
            UntypedDocumentHandle::as_container_mut,
        );
        let Some(part) = container
            .map(|c| c.parts())
            .into_iter()
            .flatten()
            .find(|p: &Part| p.id == *last)
        else {
            return Ok(None);
        };
        Ok(self.registry.decode(part.bytes, &part.hint).await.ok())
    }

    /// Convenience: [`analyze`] then [`anonymize_with`] with no editing
    /// step, redact a whole set of documents in one call. Returns the applied
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
        mut documents: impl AsDocuments,
        directives: &Directives,
    ) -> Result<Report> {
        // Resolve the slice once, then reuse it across both phases (a
        // `&mut [Document]` is itself `AsDocuments`).
        let documents = documents.as_documents_mut();
        let analyzed = self.analyze(&mut *documents, directives).await?;
        self.anonymize_with(&mut *documents, analyzed.report).await
    }

    /// Reconstruct a [`Report`] from its serialized wire form, routing each
    /// group back to the modality that produced it.
    ///
    /// The serialized report tags each group with its modality name but not the
    /// concrete type, and deserialization is not object-safe, so the report
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
    /// pipeline): an *empty* such group is skipped, the part could not have been
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

    /// Reconstruct an [`ArtifactSet`] from a serialized payload, rebuilding each
    /// group's enrichment against the registered modalities (keyed by name).
    ///
    /// The counterpart to [`deserialize_report`](Self::deserialize_report) for
    /// the enrichment [`analyze`](Self::analyze) returns beside the report:
    /// serialize [`AnalyzedDocument::artifacts`], ship it across the review gap,
    /// then deserialize it back here and pass it to
    /// [`re_analyze`](Self::re_analyze) so the OCR/transcript is reused rather
    /// than recomputed. Both ends configure the same modalities.
    ///
    /// A group naming a modality this orchestrator has no pipeline for is
    /// skipped: without a parser its enrichment cannot be rebuilt, and a re-run
    /// that lacks it simply re-enriches that group.
    ///
    /// # Errors
    ///
    /// Returns a [`MalformedInput`] error if the payload is not a valid artifact
    /// set.
    ///
    /// [`analyze`]: Self::analyze
    /// [`re_analyze`]: Self::re_analyze
    /// [`AnalyzedDocument::artifacts`]: crate::AnalyzedDocument::artifacts
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub fn deserialize_artifacts<'de, D>(&self, deserializer: D) -> Result<ArtifactSet>
    where
        D: serde::Deserializer<'de>,
    {
        self.groups.deserialize_artifacts(deserializer)
    }
}

/// The deepest (longest-path) parent key still pending in the fold map, so a
/// nested container is always resolved before the level that needs its bytes.
/// `None` when the map is empty.
fn deepest_parent<V>(by_parent: &HashMap<PartId, V>) -> Option<PartId> {
    by_parent.keys().max_by_key(|id| id.depth()).cloned()
}

/// Remove and return a container's *own* staged redacted bytes from the fold
/// map: the replacement of `local` in `parent`, if one is pending. Used so a
/// nested container is re-decoded from its own redaction rather than the
/// original, and that staged entry is not also applied verbatim (which would
/// clobber the fold). The other replacements under `parent` are left in place.
fn take_staged(
    by_parent: &mut HashMap<PartId, Vec<(LocalId, Bytes)>>,
    parent: &PartId,
    local: &LocalId,
) -> Option<Bytes> {
    let entries = by_parent.get_mut(parent)?;
    let pos = entries.iter().position(|(l, _)| l == local)?;
    let (_, bytes) = entries.remove(pos);
    if entries.is_empty() {
        by_parent.remove(parent);
    }
    Some(bytes)
}

/// One document the engine redacts: its name, the first segment of every
/// [`PartId`] beneath it, and its decoded handle.
///
/// A report describes a slice of these, analyzed and redacted as one logical
/// unit ([`analyze`] / [`anonymize_with`]); a single document is a one-element
/// slice. The name keys the document's own content (a depth-1 part) and prefixes
/// its parts' paths, so two documents that share a local part id (two scans,
/// each `page-1.png`) stay distinct, the collision a flat id would hit.
///
/// The name is the engine's, not the codec's: [`UntypedDocumentHandle`] is bytes
/// and format, never a filename, so identity is attached here, one layer up.
///
/// [`analyze`]: Orchestrator::analyze
/// [`anonymize_with`]: Orchestrator::anonymize_with
pub struct Document {
    /// The document's name, the first path segment of every part beneath it,
    /// and the key of its own content in the report. Must be unique within the
    /// slice.
    pub name: String,
    /// The document's decoded handle. Redacted in place, ready for its own
    /// `encode`.
    pub handle: UntypedDocumentHandle,
}

impl Document {
    /// A document from a name and an already-decoded handle.
    ///
    /// For the common case of decoding raw bytes into a document in one step,
    /// prefer [`FormatRegistry::document`] / [`FormatRegistry::document_with`]
    /// (the [`RegistryDocumentExt`] methods), which decode and name together.
    ///
    /// [`FormatRegistry::document`]: RegistryDocumentExt::document
    /// [`FormatRegistry::document_with`]: RegistryDocumentExt::document_with
    pub fn new(name: impl Into<String>, handle: UntypedDocumentHandle) -> Self {
        Self {
            name: name.into(),
            handle,
        }
    }
}

/// Seals [`AsDocuments`] and [`RegistryDocumentExt`] so neither is a public
/// extension point, the engine owns their whole impl surface.
mod sealed {
    pub trait Sealed {}
}

/// One or many [`Document`]s, so [`analyze`], [`re_analyze`], [`anonymize_with`],
/// and [`anonymize`] accept a single `&mut Document` or a `&mut [Document]`
/// interchangeably: a single document is a one-element slice.
///
/// Sealed, implemented only for [`Document`] and `[Document]`.
///
/// [`analyze`]: Orchestrator::analyze
/// [`re_analyze`]: Orchestrator::re_analyze
/// [`anonymize_with`]: Orchestrator::anonymize_with
/// [`anonymize`]: Orchestrator::anonymize
pub trait AsDocuments: sealed::Sealed {
    /// The documents as a mutable slice, one element for a single document,
    /// the slice itself for many.
    fn as_documents_mut(&mut self) -> &mut [Document];
}

impl sealed::Sealed for Document {}
impl sealed::Sealed for [Document] {}
impl<const N: usize> sealed::Sealed for [Document; N] {}
impl<T: sealed::Sealed + ?Sized> sealed::Sealed for &mut T {}

impl AsDocuments for Document {
    fn as_documents_mut(&mut self) -> &mut [Document] {
        std::slice::from_mut(self)
    }
}

impl AsDocuments for [Document] {
    fn as_documents_mut(&mut self) -> &mut [Document] {
        self
    }
}

impl<const N: usize> AsDocuments for [Document; N] {
    fn as_documents_mut(&mut self) -> &mut [Document] {
        self
    }
}

/// A `&mut` to anything that is [`AsDocuments`] is too, so a caller holding a
/// `&mut Document` or `&mut [Document]` (as the two-phase [`anonymize`] does
/// internally) passes it straight through.
///
/// [`anonymize`]: Orchestrator::anonymize
impl<T: AsDocuments + ?Sized> AsDocuments for &mut T {
    fn as_documents_mut(&mut self) -> &mut [Document] {
        (**self).as_documents_mut()
    }
}

/// Decode raw bytes straight into a named [`Document`], an extension trait on
/// [`FormatRegistry`], so the codec stays byte-and-format only (a handle carries
/// no filename) while the engine attaches the name it owns.
///
/// [`document`] infers the format from the name's own extension (a real filename
/// like `report.docx`); [`document_with`] takes the format explicitly, for a name
/// that carries none or a misleading one.
///
/// Sealed, implemented only for [`FormatRegistry`].
///
/// [`document`]: Self::document
/// [`document_with`]: Self::document_with
pub trait RegistryDocumentExt: sealed::Sealed {
    /// Decode `bytes` into a [`Document`] named `name`, resolving the format from
    /// the extension of `name` itself (`report.docx` → `docx`).
    ///
    /// # Errors
    ///
    /// Returns [`MalformedInput`] when `name` carries no extension to resolve a
    /// format from, use [`document_with`] with an explicit one. Otherwise
    /// propagates the decode error (e.g. [`CapabilityUnavailable`] for an
    /// unregistered format).
    ///
    /// [`document_with`]: Self::document_with
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    /// [`CapabilityUnavailable`]: elide_core::ErrorKind::CapabilityUnavailable
    fn document(
        &self,
        name: impl Into<String>,
        bytes: impl Into<ContentData>,
    ) -> impl std::future::Future<Output = Result<Document>>;

    /// Decode `bytes` into a [`Document`] named `name`, resolving the format from
    /// the explicit `extension` (which always wins, whatever `name` looks like).
    ///
    /// # Errors
    ///
    /// Propagates the decode error (e.g. [`CapabilityUnavailable`] when no format
    /// is registered for `extension`).
    ///
    /// [`CapabilityUnavailable`]: elide_core::ErrorKind::CapabilityUnavailable
    fn document_with(
        &self,
        name: impl Into<String>,
        extension: &str,
        bytes: impl Into<ContentData>,
    ) -> impl std::future::Future<Output = Result<Document>>;
}

impl sealed::Sealed for FormatRegistry {}

impl RegistryDocumentExt for FormatRegistry {
    async fn document(
        &self,
        name: impl Into<String>,
        bytes: impl Into<ContentData>,
    ) -> Result<Document> {
        let name = name.into();
        // The format is the name's own extension, lowercased, resolved via
        // `Path::extension` (which ignores a leading-dot dotfile like `.rels`).
        let extension = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let Some(extension) = extension else {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!(
                    "document name `{name}` has no extension to resolve a format from; \
                     use `document_with` with an explicit extension"
                ),
            ));
        };
        let handle = self.decode(bytes, &extension).await?;
        Ok(Document::new(name, handle))
    }

    async fn document_with(
        &self,
        name: impl Into<String>,
        extension: &str,
        bytes: impl Into<ContentData>,
    ) -> Result<Document> {
        let handle = self.decode(bytes, extension).await?;
        Ok(Document::new(name, handle))
    }
}

/// Resolve the document a path descent starts from, and the segments still to
/// walk within it: the leading segment selects the document, the remaining
/// segments walk it. `None` when no document matches the leading segment, or the
/// path is empty.
///
/// Shared by [`decode_by_path`](Orchestrator::decode_by_path) so a path resolves
/// its starting container the same way everywhere.
fn root_container<'d, 'seg>(
    documents: &'d mut [Document],
    segments: &'seg [&str],
) -> Option<(&'d mut UntypedDocumentHandle, &'seg [&'seg str])> {
    let (name, rest) = segments.split_first()?;
    let document = documents.iter_mut().find(|d| d.name == *name)?;
    Some((&mut document.handle, rest))
}

/// The *top* container named by `parent`, if `parent` is a top-level path, a
/// one-segment document path. `None` for a deeper parent (a nested container to
/// re-decode). The fold writes straight into a top container, which re-encodes
/// itself.
fn top_container<'d>(
    documents: &'d mut [Document],
    parent: &PartId,
) -> Option<&'d mut UntypedDocumentHandle> {
    let mut segments = parent.segments();
    let name = segments.next()?;
    // A top document is exactly a one-segment path; anything deeper is a nested
    // container, not a root.
    if segments.next().is_some() {
        return None;
    }
    documents
        .iter_mut()
        .find(|d| d.name == name)
        .map(|d| &mut d.handle)
}
