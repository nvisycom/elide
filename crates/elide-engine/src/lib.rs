#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analysis;
mod directives;
mod document;
mod part_id;
mod pipeline;

use std::any::TypeId;
use std::collections::HashMap;

use bytes::Bytes;
use elide_codec::{Document as CodecDocument, DocumentPart, ErasedStream, LocalId, TypedStream};
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, NoArtifact, StreamDataReader};
use elide_core::recognition::Scope;
use elide_core::{Error, ErrorKind, Result};
use elide_detection::Analyzer;
use elide_format::FormatRegistry;
use elide_redaction::Anonymizer;

pub use self::analysis::{AnalyzedDocument, ArtifactSet, Report, ReportDeserializer};
// `EntityGroup` / `ArtifactGroup` are the crate-internal erased storage the
// report and artifact set hold; the public construction bounds are expressed in
// terms of `serde::Serialize`, which the blanket impls satisfy, so neither trait
// is named in any public signature and both stay `pub(crate)`.
use self::analysis::{ArtifactGroup, ModalityRegistry, PartReport};
use self::directives::AnnotationSet;
pub use self::directives::Directives;
pub use self::document::{AsDocuments, Document, RegistryDocumentExt};
pub use self::part_id::PartId;
use self::pipeline::{BoxFuture, ErasedPipeline, ModalityPipeline};

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
    /// [`FormatRegistry::with_builtin`]: elide_format::FormatRegistry::with_builtin
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
        TypedStream<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
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
        TypedStream<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
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
        TypedStream<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
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
        TypedStream<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
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

        // Each document is a tree of parts keyed under its own name, so two
        // documents sharing a local part id stay distinct paths. Its stream
        // parts are analyzed in place; its blob sub-parts decode into their own
        // child documents and recurse.
        for document in documents.iter_mut() {
            let prefix = PartId::leaf(document.name.clone());
            self.analyze_parts(
                &mut document.document,
                &prefix,
                0,
                prior,
                scope,
                annotations,
                &mut report,
                &mut artifacts,
            )
            .await?;
        }

        Ok(AnalyzedDocument { report, artifacts })
    }

    /// Analyze one decoded [`CodecDocument`]'s parts, keyed under `prefix`
    /// (the document's tree path). Its *body* — the first
    /// [`Stream`](DocumentPart::Stream) part — is the document's own content,
    /// keyed at `prefix` itself (a leaf document's whole content, a container's
    /// body text); every other part keys at `prefix.child(id)`. Each stream is
    /// offered to every pipeline until one matches its modality, then analyzed in
    /// place; a part whose modality no pipeline covers passes through untouched.
    /// Each [`Blob`](DocumentPart::Blob) sub-part is decoded through the registry
    /// into its own child document and recursed into (depth `+ 1`); a blob no
    /// codec can decode is opaque, left as-is.
    ///
    /// Recurses so an arbitrarily nested tree (an image in a DOCX embedded in a
    /// bundle) is reached in full. Past [`MAX_CONTAINER_DEPTH`] a nested document
    /// is a hard error, never a silent stop, so nothing nested is left
    /// un-redacted.
    #[allow(clippy::too_many_arguments)]
    fn analyze_parts<'a>(
        &'a self,
        document: &'a mut CodecDocument,
        prefix: &'a PartId,
        depth: usize,
        prior: &'a ArtifactSet,
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
        report: &'a mut Report,
        artifacts: &'a mut ArtifactSet,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let mut body_seen = false;
            for part in document.parts_mut() {
                match part {
                    DocumentPart::Stream { id, handle } => {
                        let part_id = body_part_id(prefix, id, &mut body_seen);
                        self.analyze_stream_into(
                            handle,
                            part_id,
                            prior,
                            scope,
                            annotations,
                            report,
                            artifacts,
                        )
                        .await?;
                    }
                    DocumentPart::Blob { id, bytes, hint } => {
                        let part_id = prefix.child(id.clone());
                        // Past the depth bound a nested document is a hard error,
                        // never a silent drop.
                        if depth + 1 > MAX_CONTAINER_DEPTH {
                            return Err(Error::new(
                                ErrorKind::MalformedInput,
                                format!(
                                    "container nesting exceeds the depth limit of \
                                     {MAX_CONTAINER_DEPTH} at part `{part_id}`"
                                ),
                            ));
                        }
                        let Ok(mut child) = self.registry.decode(bytes.clone(), hint).await else {
                            continue; // no codec for this blob, opaque, left as-is
                        };
                        self.analyze_parts(
                            &mut child,
                            &part_id,
                            depth + 1,
                            prior,
                            scope,
                            annotations,
                            report,
                            artifacts,
                        )
                        .await?;
                    }
                }
            }
            Ok(())
        })
    }

    /// Analyze one stream part in place, offering it to each pipeline until one
    /// matches its modality, seeded with its prior enrichment. The findings and
    /// any enrichment artifact are stored under `id`. A stream whose modality no
    /// pipeline covers stores nothing (an intentional pass-through).
    #[allow(clippy::too_many_arguments)]
    async fn analyze_stream_into(
        &self,
        handle: &mut ErasedStream,
        id: PartId,
        prior: &ArtifactSet,
        scope: &Scope,
        annotations: &AnnotationSet,
        report: &mut Report,
        artifacts: &mut ArtifactSet,
    ) -> Result<()> {
        let empty: Box<dyn ArtifactGroup> = Box::new(NoArtifact);
        let seed = prior
            .parts
            .get(&id)
            .map_or(empty.as_ref(), |e| e.artifact.as_ref());
        for (modality, pipeline) in &self.pipelines {
            let Some(analyzed) = pipeline
                .analyze_stream(handle, scope, annotations, seed)
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

    /// Apply a (possibly edited) [`Report`] back onto the same `documents`:
    /// redact each stream part in place and fold each blob sub-part's redacted
    /// bytes back up its document's part tree, then re-encode each document
    /// (`document.encode()`) to serialize the result. `documents` must be the
    /// same set the report describes.
    ///
    /// A stream part is redacted in place through the pipeline for its modality;
    /// a blob sub-part is decoded into its own child document, redacted the same
    /// way, re-encoded, and folded back into its parent via
    /// [`replace_part`](CodecDocument::replace_part). A document's own
    /// [`encode`](CodecDocument::encode) then assembles the redacted parts
    /// post-order, so a nested document re-encodes (carrying its *own* body
    /// redaction) before the level above assembles it.
    ///
    /// Returns the report, now applied: redaction stamps a redaction event into
    /// each entity's provenance, so the returned report's entities carry the full
    /// audit trail (recognition through redaction), serialize it to hand the
    /// audit to a caller.
    ///
    /// [`analyze`]: Self::analyze
    pub async fn anonymize_with(
        &self,
        mut documents: impl AsDocuments,
        mut report: Report,
    ) -> Result<Report> {
        let documents = documents.as_documents_mut();
        // Applying mutates the entities (each gains a redaction event), so it
        // happens on `report`'s own groups, which are returned as the audit
        // trail. Each document redacts its stream parts in place and folds its
        // blob sub-parts bottom-up through the recursion.
        for document in documents.iter_mut() {
            let prefix = PartId::leaf(document.name.clone());
            self.apply_parts(&mut document.document, &prefix, &mut report)
                .await?;
        }
        Ok(report)
    }

    /// Apply the report to one decoded [`CodecDocument`]'s parts, keyed under
    /// `prefix`. Each [`Stream`](DocumentPart::Stream) part with a report entry
    /// is redacted in place through the pipeline for its modality. Each
    /// [`Blob`](DocumentPart::Blob) sub-part is decoded into its own child
    /// document, that child recursed into (applying any report parts beneath it),
    /// re-encoded, and folded back via
    /// [`replace_part`](CodecDocument::replace_part) — post-order, so a nested
    /// document re-encodes (carrying its own body redaction) before its parent
    /// assembles it.
    ///
    /// A blob that no codec can decode, or has no redacted descendant, is left
    /// as-is (its original bytes fold through unchanged).
    fn apply_parts<'a>(
        &'a self,
        document: &'a mut CodecDocument,
        prefix: &'a PartId,
        report: &'a mut Report,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // Blob sub-parts fold back after the walk: `replace_part` needs a
            // fresh `&mut document`, which the `parts_mut` borrow holds for the
            // loop, so stage each blob's redacted bytes and apply them after.
            let mut folded: Vec<(LocalId, Bytes)> = Vec::new();
            let mut body_seen = false;
            for part in document.parts_mut() {
                match part {
                    DocumentPart::Stream { id, handle } => {
                        let part_id = body_part_id(prefix, id, &mut body_seen);
                        let Some(entry) = report.parts.get_mut(&part_id) else {
                            continue; // no findings for this stream
                        };
                        let Some(pipeline) = self.pipelines.get(&entry.modality) else {
                            continue; // pipeline for this modality is gone
                        };
                        pipeline
                            .apply_stream(handle, entry.entities.as_mut(), &self.scope)
                            .await?;
                    }
                    DocumentPart::Blob { id, bytes, hint } => {
                        let part_id = prefix.child(id.clone());
                        let Ok(mut child) = self.registry.decode(bytes.clone(), hint).await else {
                            continue; // no codec for this blob, opaque, left as-is
                        };
                        // Recurse first (post-order): the child re-encodes its own
                        // redacted streams, then folds back into this document.
                        self.apply_parts(&mut child, &part_id, report).await?;
                        folded.push((id.clone(), child.encode()?.into_bytes()));
                    }
                }
            }
            for (id, bytes) in folded {
                document.replace_part(&id, bytes)?;
            }
            Ok(())
        })
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

/// The [`PartId`] under which a document part is keyed. The document's *body* —
/// its first [`Stream`](DocumentPart::Stream) part — is the document's own
/// content, keyed at `prefix` itself (depth 1 for a top-level document); every
/// other part keys at `prefix.child(id)`. `body_seen` tracks whether the body
/// has been claimed, so the first stream wins it and later parts nest beneath.
fn body_part_id(prefix: &PartId, id: &LocalId, body_seen: &mut bool) -> PartId {
    if *body_seen {
        prefix.child(id.clone())
    } else {
        *body_seen = true;
        prefix.clone()
    }
}
