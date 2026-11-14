//! The two [`PartVisitor`]s the part-walk drives: [`Analyze`] (batch the stream
//! leaves, scatter findings, never fold) and [`Redact`] (apply each stream's
//! report entry in place, fold changed blob children back, remove unreadable
//! ones).

use std::collections::HashSet;

use elide_codec::StreamMut;
use elide_core::modality::NoArtifact;
use elide_core::recognition::Scope;
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};

use super::{Child, Fold, PartVisitor};
use crate::Orchestrator;
use crate::analysis::{ArtifactGroup, ArtifactSet, Report, StrippedRecord};
use crate::part_id::PartId;
use crate::pipeline::{BoxFuture, PartAnalysis};

/// The analysis phase: batch each level's streams through the pipeline for
/// their modality and scatter the findings and enrichment artifact into the
/// report, keyed by part id. Blob children never fold back (analysis mutates the
/// report, not the document).
pub(crate) struct Analyze<'p> {
    /// Prior enrichment to seed each part with (a re-run's restored artifacts, an
    /// empty set on a first pass).
    pub(crate) prior: &'p ArtifactSet,
    /// The orchestrator's analysis scope.
    pub(crate) scope: &'p Scope,
    /// The report this run writes its findings into.
    pub(crate) report: &'p mut Report,
    /// The enrichment artifacts this run produces.
    pub(crate) artifacts: &'p mut ArtifactSet,
    /// The hidden content this run sees stripped.
    pub(crate) stripped: StrippedRecord,
}

impl PartVisitor for Analyze<'_> {
    fn visit_streams<'a>(
        &'a mut self,
        orchestrator: &'a Orchestrator,
        mut streams: Vec<(PartId, StreamMut<'a>)>,
        policy: HiddenContentPolicy,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            let no_artifact: Box<dyn ArtifactGroup> = Box::new(NoArtifact);
            // Same-modality parts analyze together, so an OCR/STT enricher issues
            // one batched provider round-trip across them. Each pipeline claims the
            // handles it covers; leftovers have no covering pipeline (pass-through).
            for (modality, pipeline) in orchestrator.pipelines() {
                let (mine, rest): (Vec<_>, Vec<_>) = streams
                    .into_iter()
                    .partition(|(_, handle)| pipeline.matches(handle));
                streams = rest;
                if mine.is_empty() {
                    continue;
                }
                let (ids, mut handles): (Vec<PartId>, Vec<StreamMut<'_>>) =
                    mine.into_iter().unzip();
                let seeds: Vec<&dyn ArtifactGroup> = ids
                    .iter()
                    .map(|id| {
                        self.prior
                            .parts
                            .get(id)
                            .map_or(no_artifact.as_ref(), |e| e.artifact.as_ref())
                    })
                    .collect();
                let analyses = pipeline
                    .analyze_streams(&mut handles, self.scope, policy, &seeds)
                    .await?;
                for (id, PartAnalysis { entities, artifact }) in ids.into_iter().zip(analyses) {
                    let name = entities.modality_name();
                    self.report.parts.insert(id.clone(), entities);
                    if let Some(artifact) = artifact {
                        self.artifacts.set_part(id, *modality, name, artifact);
                    }
                }
            }
            // Analysis writes the report, not the document, so nothing "changed".
            Ok(false)
        })
    }

    fn fold_child(&mut self, _child: Child<'_>) -> Result<Fold> {
        Ok(Fold::Keep)
    }

    fn stripped(&mut self) -> &mut StrippedRecord {
        &mut self.stripped
    }
}

/// The redaction phase: redact each stream in place through its report
/// entry, fold a changed blob child's re-encoded bytes back into its parent,
/// and remove a blob no codec can read.
pub(crate) struct Redact<'p> {
    /// The (possibly edited) report to apply; entities gain a redaction event.
    pub(crate) report: &'p mut Report,
    /// The hidden content this run sees stripped.
    pub(crate) stripped: StrippedRecord,
    /// The report parts this run applied, so a part the walk never reached is
    /// caught rather than silently left unredacted.
    pub(crate) applied: HashSet<PartId>,
}

impl PartVisitor for Redact<'_> {
    fn visit_streams<'a>(
        &'a mut self,
        orchestrator: &'a Orchestrator,
        streams: Vec<(PartId, StreamMut<'a>)>,
        _policy: HiddenContentPolicy,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            let mut changed = false;
            for (part_id, mut handle) in streams {
                let Some(entities) = self.report.parts.get_mut(&part_id) else {
                    continue;
                };
                let Some(pipeline) = orchestrator.pipeline_for(entities.modality()) else {
                    // Nothing to apply is nothing lost; entities with no pipeline
                    // to apply them would ship the part unredacted.
                    if entities.is_empty() {
                        continue;
                    }
                    return Err(Error::malformed(format!(
                        "no pipeline is registered for the {} entities of part `{part_id}`",
                        entities.modality_name()
                    )));
                };
                changed |= pipeline
                    .apply_stream(&mut handle, entities.as_mut(), orchestrator.scope())
                    .await?;
                self.applied.insert(part_id);
            }
            Ok(changed)
        })
    }

    fn fold_child(&mut self, child: Child<'_>) -> Result<Fold> {
        // A changed child re-encodes its own redacted subtree and folds back; an
        // unchanged one keeps its parent's original blob bytes, so an untouched
        // embedding is never re-serialized; one no codec can read is removed.
        Ok(match child {
            Child::Decoded {
                document,
                changed: true,
            } => Fold::Replace(document.encode()?.into_bytes()),
            Child::Decoded { changed: false, .. } => Fold::Keep,
            Child::Unredactable => Fold::Remove,
        })
    }

    fn stripped(&mut self) -> &mut StrippedRecord {
        &mut self.stripped
    }
}
