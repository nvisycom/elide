#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc = include_str!("../README.md")]

mod analysis;
mod document;
mod part_id;
mod pipeline;
mod traversal;

use std::any::TypeId;
use std::collections::HashMap;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::modality::text::Text;
use elide_core::recognition::Scope;
use elide_core::redaction::Operator;
use elide_core::{Error, Result};
use elide_detection::Analyzer;
use elide_format::FormatRegistry;
use elide_redaction::Anonymizer;
use elide_redaction::operators::Erase;

pub use self::analysis::{
    AnalyzedDocument, ArtifactSet, Part, PartMut, Report, ReportDeserializer, StrippedItem,
};
use self::analysis::{ModalityRegistry, StrippedRecord};
pub use self::document::{AsDocuments, Document, RegistryDocumentExt};
pub use self::part_id::PartId;
use self::pipeline::{ErasedPipeline, ModalityPipeline, metadata};
use self::traversal::{Analyze, Redact, Walk};

/// Drives detection and redaction across a set of documents, part by part.
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
/// Every orchestrator drives [`Metadata`] without being configured: a field's
/// format labels it, so each surfaced field the scope's catalog declares (every
/// one, under [`Disposition::FlagEverything`]) is detected, and erased unless
/// [`with_anonymizer`] sets another treatment. Under `FlagEverything` for
/// collaboration, each comment, author, and tracked deletion a format labels is
/// an entity too, so every orchestrator also drives text, with no recognizers
/// until [`with_analyzer`] or [`with_modality`] sets some. Every pipeline's
/// anonymizer ends in a rule erasing flagged hidden content its own rules do not
/// cover, so a flagged item is redacted unless a caller rule (a [`Keep`], say)
/// matches it first.
///
/// [`with_modality`]: Orchestrator::with_modality
/// [`with_registry`]: Orchestrator::with_registry
/// [`analyze`]: Orchestrator::analyze
/// [`anonymize_with`]: Orchestrator::anonymize_with
/// [`anonymize`]: Orchestrator::anonymize
/// [`with_anonymizer`]: Orchestrator::with_anonymizer
/// [`with_analyzer`]: Orchestrator::with_analyzer
/// [`Keep`]: elide_redaction::operators::Keep
/// [`Metadata`]: elide_core::modality::metadata::Metadata
/// [`Disposition::FlagEverything`]: elide_core::redaction::Disposition::FlagEverything
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

impl Default for Orchestrator {
    fn default() -> Self {
        let mut orchestrator = Self {
            registry: FormatRegistry::default(),
            pipelines: HashMap::new(),
            groups: ModalityRegistry::default(),
            scope: Scope::default(),
        };
        orchestrator.insert_pipeline(metadata::pipeline());
        orchestrator.insert_pipeline(ModalityPipeline::<Text>::new(
            Analyzer::new(),
            Anonymizer::new(),
        ));
        orchestrator
    }
}

impl Orchestrator {
    /// A new orchestrator: only the built-in [`Metadata`] pipeline and a [`Text`]
    /// pipeline that detects nothing and erases only flagged hidden content, an
    /// empty [`Scope`], and an empty [`FormatRegistry`] (so container parts do
    /// not decode until one is supplied via [`with_registry`]).
    ///
    /// [`with_registry`]: Self::with_registry
    /// [`Text`]: elide_core::modality::text::Text
    /// [`Metadata`]: elide_core::modality::metadata::Metadata
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the [`FormatRegistry`] that decodes a document's container parts.
    ///
    /// A body-only document needs no registry; a container (DOCX, …) needs one
    /// that covers its part formats, typically
    /// [`FormatRegistry::with_builtin`].
    ///
    /// [`FormatRegistry::with_builtin`]: elide_format::FormatRegistry::with_builtin
    #[must_use]
    pub fn with_registry(mut self, registry: FormatRegistry) -> Self {
        self.registry = registry;
        self
    }

    /// Set the run-wide default [`Scope`], shared by every modality pipeline.
    ///
    /// The caller's analysis-wide assertions: languages, jurisdictions, tags,
    /// catalog, correlation id.
    ///
    /// A `Scope` is modality-free, so one drives the body and every
    /// container part alike; no need to repeat it per [`with_modality`].
    ///
    /// [`with_modality`]: Self::with_modality
    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// Register the detection and redaction pipeline for modality `M`.
    ///
    /// A part that decodes to `M` is driven by this pipeline; parts of a
    /// modality with no registered pipeline pass through untouched.
    /// Re-registering a modality replaces both halves. For [`Metadata`] the
    /// halves add to the flagged fields rather than replace them.
    ///
    /// When a modality only ever detects *or* only ever redacts, register just
    /// the half it needs with [`with_analyzer`] / [`with_anonymizer`] instead,
    /// the other half defaults to a no-op, which still erases flagged hidden
    /// content.
    ///
    /// [`with_analyzer`]: Self::with_analyzer
    /// [`with_anonymizer`]: Self::with_anonymizer
    /// [`Metadata`]: elide_core::modality::metadata::Metadata
    #[must_use]
    pub fn with_modality<M>(mut self, analyzer: Analyzer<M>, anonymizer: Anonymizer<M>) -> Self
    where
        M: Modality,
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
        Erase: Operator<M>,
    {
        match self.modality_pipeline_mut::<M>() {
            Some(pipeline) => {
                pipeline.analyzer = analyzer;
                pipeline.set_anonymizer(anonymizer);
            }
            None => self.insert_pipeline(ModalityPipeline::new(analyzer, anonymizer)),
        }
        self
    }

    /// Register or update only the *analyzer* for modality `M`, the detection
    /// half.
    ///
    /// If a pipeline for `M` already exists, its analyzer is replaced and its
    /// anonymizer kept; otherwise a new pipeline is created with a no-op
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
        Erase: Operator<M>,
    {
        match self.modality_pipeline_mut::<M>() {
            Some(pipeline) => pipeline.analyzer = analyzer,
            None => self.insert_pipeline(ModalityPipeline::new(analyzer, Anonymizer::new())),
        }
        self
    }

    /// Register or update only the *anonymizer* for modality `M`, the redaction
    /// half.
    ///
    /// If a pipeline for `M` already exists, its anonymizer is replaced and its
    /// analyzer kept; otherwise a new pipeline is created with a no-op
    /// [`Analyzer::new`] as the detection half.
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
        Erase: Operator<M>,
    {
        match self.modality_pipeline_mut::<M>() {
            Some(pipeline) => pipeline.set_anonymizer(anonymizer),
            None => self.insert_pipeline(ModalityPipeline::new(Analyzer::new(), anonymizer)),
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

    /// Insert `pipeline` for `M` and register its deserialize parser.
    fn insert_pipeline<M>(&mut self, pipeline: ModalityPipeline<M>)
    where
        M: Modality,
        Vec<Entity<M>>: serde::Serialize + serde::de::DeserializeOwned,
        M::Artifact: serde::Serialize + serde::de::DeserializeOwned,
    {
        self.pipelines.insert(TypeId::of::<M>(), Box::new(pipeline));
        self.groups.register::<M>();
    }

    /// The registered pipelines, keyed by modality [`TypeId`], for the part-walk
    /// phases to classify and drive streams.
    pub(crate) fn pipelines(&self) -> impl Iterator<Item = (&TypeId, &Box<dyn ErasedPipeline>)> {
        self.pipelines.iter()
    }

    /// The pipeline for `modality`, or `None` when none is registered for it.
    pub(crate) fn pipeline_for(&self, modality: TypeId) -> Option<&dyn ErasedPipeline> {
        self.pipelines.get(&modality).map(Box::as_ref)
    }

    /// The orchestrator's default scope, used by the redaction phase.
    pub(crate) fn scope(&self) -> &Scope {
        &self.scope
    }

    /// The format registry used to decode container blobs, for the part-walk.
    pub(crate) fn registry(&self) -> &FormatRegistry {
        &self.registry
    }

    /// Detect the entities of a set of documents, without redacting them.
    ///
    /// Covers each [`Document`]'s own content *and* every container part whose
    /// modality has a registered pipeline. A single document is a one-element
    /// slice; a scan stack or a batch shipped together is redacted as one
    /// logical unit, so entities are found (and later removed) consistently
    /// across all of them. Returns an [`AnalyzedDocument`]: its editable
    /// [`report`] to hand to [`anonymize_with`], and the [`artifacts`] (the
    /// OCR/transcript enrichment) to persist across a review gap and pass to
    /// [`re_analyze`], which reuses them instead of re-enriching.
    ///
    /// The orchestrator's [`with_scope`] applies to the whole set.
    ///
    /// Every document is a top-level file: its own content is analyzed and keyed
    /// under its [`name`](Document::name) as a depth-1 part, and its container
    /// parts are flattened beneath that name, so two files sharing a local part
    /// id (two scans, each `page-1.png`) never collide. Each document, and each
    /// part, is offered to every pipeline until one matches its modality; that
    /// pipeline analyzes it. A document or part with no matching pipeline (or
    /// that no codec can decode) passes through untouched.
    ///
    /// Read a part with [`Report::part`] keyed by its name, and edit it with
    /// [`Report::part_mut`] before applying. For the common single-document
    /// case, [`Report::document`] reads the sole document directly.
    ///
    /// [`with_scope`]: Self::with_scope
    /// [`anonymize_with`]: Self::anonymize_with
    /// [`re_analyze`]: Self::re_analyze
    /// [`report`]: AnalyzedDocument::report
    /// [`artifacts`]: AnalyzedDocument::artifacts
    pub async fn analyze(&self, mut documents: impl AsDocuments) -> Result<AnalyzedDocument> {
        // A first pass is a re-run seeded with nothing: every document and part
        // starts from the empty (default) artifact, so it enriches from scratch.
        // Sharing the driver keeps the two entry points from drifting.
        self.drive(documents.as_documents_mut(), &ArtifactSet::new())
            .await
    }

    /// Re-detect over `documents`, reusing the enrichment from a `prior`
    /// analysis.
    ///
    /// Each group's recognition is seeded with the artifact from `prior`, so
    /// the OCR/transcript is reused rather than recomputed. The re-run
    /// counterpart to [`analyze`](Self::analyze), for detection separated in
    /// time from a first pass: after the review gap, re-run recognition (with
    /// another recognizer, say) without paying for OCR/STT again.
    ///
    /// A group `prior` has no artifact for, a document or part it never
    /// analyzed, re-runs from an empty (default) artifact, i.e. it re-enriches.
    /// `documents` must be the same set the `prior` artifacts were produced from.
    pub async fn re_analyze(
        &self,
        mut documents: impl AsDocuments,
        prior: &ArtifactSet,
    ) -> Result<AnalyzedDocument> {
        self.drive(documents.as_documents_mut(), prior).await
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
    ) -> Result<AnalyzedDocument> {
        let mut report = Report::new();
        let mut artifacts = ArtifactSet::new();

        // A document's name is its depth-1 `PartId`, the key its own content and
        // every nested part hang under. Two documents sharing a name would key to
        // the same path, so the second would overwrite the first in the report and
        // never be reached at apply. Reject the collision up front rather than
        // silently drop a document's redaction.
        let mut names = std::collections::HashSet::new();
        for document in documents.iter() {
            if !names.insert(document.name.as_str()) {
                return Err(Error::malformed(format!(
                    "duplicate document name `{}` in the set",
                    document.name
                )));
            }
        }

        // Each document is a tree of parts keyed under its own name, so two
        // documents sharing a local part id stay distinct paths. The analysis
        // phase batches each level's same-modality streams and scatters the
        // findings; blobs decode into their own child documents and
        // recurse.
        let mut visitor = Analyze {
            prior,
            scope: &self.scope,
            report: &mut report,
            artifacts: &mut artifacts,
            stripped: StrippedRecord::default(),
        };
        for document in documents.iter_mut() {
            let prefix = PartId::leaf(document.name.clone());
            Walk::new(self, &mut visitor)
                .parts(&mut document.document, &prefix, 0)
                .await?;
        }
        let stripped = visitor.stripped;
        report.set_stripped(stripped);

        Ok(AnalyzedDocument { report, artifacts })
    }

    /// Apply a (possibly edited) [`Report`] back onto the same `documents`.
    ///
    /// It redacts each stream in place and folds each blob's redacted bytes
    /// back up its document's part tree; re-encode each document
    /// (`document.encode()`) to serialize the result. `documents` must be the
    /// same set the report describes.
    ///
    /// A stream is redacted in place through the pipeline for its modality;
    /// a blob is decoded into its own child document, redacted the same
    /// way, re-encoded, and folded back into its parent via its `replace_blob`. A
    /// document's own `encode` then writes it out post-order, so a nested
    /// document re-encodes (carrying its *own* body redaction) before the level
    /// above writes it.
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
        // trail. Each document redacts its streams in place and folds its
        // blobs bottom-up through the recursion.
        // The applied report's stripped section is what this walk saw, whatever
        // the report passed in carried.
        let mut visitor = Redact {
            report: &mut report,
            stripped: StrippedRecord::default(),
        };
        for document in documents.iter_mut() {
            let prefix = PartId::leaf(document.name.clone());
            Walk::new(self, &mut visitor)
                .parts(&mut document.document, &prefix, 0)
                .await?;
        }
        let stripped = visitor.stripped;
        report.set_stripped(stripped);
        Ok(report)
    }

    /// Record on every entity of `report` the operator [`anonymize_with`] will
    /// apply to it, as a `Selection` event, without touching any document.
    ///
    /// Each part is picked by the anonymizer registered for its modality, under
    /// the orchestrator's [`with_scope`], the same ones [`anonymize_with`]
    /// applies with, so the plan a reviewer sees is the plan that runs.
    /// Overlapping entities resolve to one operator, as they will at apply. A
    /// part of a modality with no pipeline is skipped, as [`anonymize_with`]
    /// passes it through. Picking again after a review edit records only what
    /// changed: an entity whose latest pick already names the same operator
    /// gains nothing.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`](elide_core::ErrorKind::MalformedInput) when a part's
    /// entities are not of the modality the report keys it under (a hand-built
    /// or deserialized report).
    ///
    /// [`anonymize_with`]: Self::anonymize_with
    /// [`with_scope`]: Self::with_scope
    pub fn pick(&self, report: &mut Report) -> Result<()> {
        for part in report.parts.values_mut() {
            if let Some(pipeline) = self.pipelines.get(&part.modality) {
                pipeline.pick(part.entities.as_mut(), &self.scope)?;
            }
        }
        Ok(())
    }

    /// Detect and redact a whole set of documents in one call, with no review.
    ///
    /// Convenience: [`analyze`] then [`anonymize_with`] with no editing step.
    /// Returns the applied [`Report`], whose entities carry the full audit
    /// trail (recognition through redaction).
    ///
    /// Use the two phases directly when you need to inspect or edit the
    /// detected entities (drop a false positive, retag) between detection
    /// and redaction.
    ///
    /// [`analyze`]: Self::analyze
    /// [`anonymize_with`]: Self::anonymize_with
    pub async fn anonymize(&self, mut documents: impl AsDocuments) -> Result<Report> {
        // Resolve the slice once, then reuse it across both phases (a
        // `&mut [Document]` is itself `AsDocuments`).
        let documents = documents.as_documents_mut();
        let analyzed = self.analyze(&mut *documents).await?;
        self.anonymize_with(&mut *documents, analyzed.report).await
    }

    /// Reconstruct a [`Report`] from its serialized form, by producing
    /// modality.
    ///
    /// Each group is routed back to the modality that produced it.
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
