//! The two [`PartVisitor`]s the part-walk drives: [`Analyze`] (batch the stream
//! leaves, scatter findings, never fold) and [`Redact`] (apply each stream's
//! report entry in place, fold changed blob children back).

use elide_codec::{Document as CodecDocument, ErasedStream};
use elide_core::Result;
use elide_core::modality::NoArtifact;
use elide_core::recognition::Scope;

use super::PartVisitor;
use crate::Orchestrator;
use crate::analysis::{ArtifactGroup, ArtifactSet, PartReport, Report};
use crate::directives::AnnotationSet;
use crate::part_id::PartId;
use crate::pipeline::BoxFuture;

/// The analysis phase: batch each level's stream parts through the pipeline for
/// their modality and scatter the findings and enrichment artifact into the
/// report, keyed by part id. Blob children never fold back (analysis mutates the
/// report, not the document).
pub(crate) struct Analyze<'p> {
    /// Prior enrichment to seed each part with (a re-run's restored artifacts, an
    /// empty set on a first pass).
    pub(crate) prior: &'p ArtifactSet,
    /// The analysis scope (a per-call override, or the orchestrator default).
    pub(crate) scope: &'p Scope,
    /// Per-request region annotations.
    pub(crate) annotations: &'p AnnotationSet,
    /// The report this run writes its findings into.
    pub(crate) report: &'p mut Report,
    /// The enrichment artifacts this run produces.
    pub(crate) artifacts: &'p mut ArtifactSet,
}

impl PartVisitor for Analyze<'_> {
    fn visit_streams<'a>(
        &'a mut self,
        orchestrator: &'a Orchestrator,
        mut streams: Vec<(PartId, &'a mut ErasedStream)>,
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
                let (ids, mut handles): (Vec<PartId>, Vec<&mut ErasedStream>) =
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
                    .analyze_streams(&mut handles, self.scope, self.annotations, &seeds)
                    .await?;
                for (id, (entities, artifact)) in ids.into_iter().zip(analyses) {
                    let name = entities.modality_name();
                    self.report.parts.insert(
                        id.clone(),
                        PartReport {
                            modality: *modality,
                            entities,
                        },
                    );
                    if let Some(artifact) = artifact {
                        self.artifacts.set_part(id, *modality, name, artifact);
                    }
                }
            }
            // Analysis writes the report, not the document, so nothing "changed".
            Ok(false)
        })
    }

    fn fold_child(
        &mut self,
        _changed: bool,
        _child: &CodecDocument,
    ) -> Result<Option<bytes::Bytes>> {
        Ok(None)
    }
}

/// The redaction phase: redact each stream part in place through its report
/// entry, marking the document changed, and fold a changed blob child's
/// re-encoded bytes back into its parent.
pub(crate) struct Redact<'p> {
    /// The (possibly edited) report to apply; entities gain a redaction event.
    pub(crate) report: &'p mut Report,
}

impl PartVisitor for Redact<'_> {
    fn visit_streams<'a>(
        &'a mut self,
        orchestrator: &'a Orchestrator,
        streams: Vec<(PartId, &'a mut ErasedStream)>,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            let mut changed = false;
            for (part_id, handle) in streams {
                let Some(entry) = self.report.parts.get_mut(&part_id) else {
                    continue; // no findings for this stream
                };
                let Some(pipeline) = orchestrator.pipeline_for(entry.modality) else {
                    continue; // pipeline for this modality is gone
                };
                pipeline
                    .apply_stream(handle, entry.entities.as_mut(), orchestrator.scope())
                    .await?;
                changed = true;
            }
            Ok(changed)
        })
    }

    fn fold_child(&mut self, changed: bool, child: &CodecDocument) -> Result<Option<bytes::Bytes>> {
        // A changed child re-encodes its own redacted subtree and folds back; an
        // unchanged one keeps its parent's original blob bytes, so an untouched
        // embedding is never re-serialized.
        if changed {
            Ok(Some(child.encode()?.into_bytes()))
        } else {
            Ok(None)
        }
    }
}
