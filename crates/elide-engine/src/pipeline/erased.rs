//! [`ErasedPipeline`]: the type-erased pipeline the [`Orchestrator`] stores per
//! modality, so a document's stream can be offered to each pipeline until
//! one matches without the orchestrator naming the modality statically.
//!
//! [`Orchestrator`]: crate::Orchestrator

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use elide_codec::StreamMut;
use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::recognition::Scope;
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Error, Result};

use super::{ModalityPipeline, flagged};
use crate::analysis::{ArtifactGroup, EntityGroup};

/// A boxed, pinned, `Send` future, the erased async return shape.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What analyzing one stream produced: its entities, and its enrichment
/// artifact when it was enriched.
pub(crate) struct PartAnalysis {
    pub(crate) entities: Box<dyn EntityGroup>,
    pub(crate) artifact: Option<Box<dyn ArtifactGroup>>,
}

/// A type-erased pipeline the orchestrator stores per modality.
///
/// Every document stream is a [`StreamMut`] the orchestrator buckets by
/// modality ([`matches`]) so it never names the modality statically. The analyze
/// phase is seeded with the group's prior enrichment artifact, `NoArtifact` on a
/// first pass, a restored artifact on a re-run, so the same path serves both. The
/// phases:
/// - [`analyze_streams`] analyzes the matched parts together, coalescing
///   enrichment into one provider round-trip, and returns one result per handle.
/// - [`apply_stream`] redacts a matched stream in place with its (possibly
///   edited) boxed entities; the document re-encodes itself.
///
/// [`matches`]: ErasedPipeline::matches
/// [`analyze_streams`]: ErasedPipeline::analyze_streams
/// [`apply_stream`]: ErasedPipeline::apply_stream
pub(crate) trait ErasedPipeline: Send + Sync {
    /// Whether `handle` carries this pipeline's modality, so the orchestrator
    /// can bucket parts by modality before dispatching a batch.
    fn matches(&self, handle: &StreamMut<'_>) -> bool;

    /// Analyze several streams of this pipeline's modality together,
    /// coalescing enrichment into one batched provider round-trip, and return one
    /// [`PartAnalysis`] per handle in order. Each handle is paired with its prior
    /// enrichment `seed` (`NoArtifact` on a first pass), and its flagged hidden
    /// content is detected under `policy`. Every handle must already
    /// match this pipeline's modality (the orchestrator groups them with
    /// [`matches`](Self::matches) first); a non-matching handle is a caller error.
    fn analyze_streams<'a>(
        &'a self,
        handles: &'a mut [StreamMut<'_>],
        scope: &'a Scope,
        policy: HiddenContentPolicy,
        seeds: &'a [&'a dyn ArtifactGroup],
    ) -> BoxFuture<'a, Result<Vec<PartAnalysis>>>;

    /// Redact a matched stream in place with its (possibly edited) boxed
    /// entities; the document re-encodes the stream itself on `encode`.
    /// Returns whether anything was written to the stream.
    fn apply_stream<'a>(
        &'a self,
        handle: &'a mut StreamMut<'_>,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<bool>>;

    /// Record on each of `entities` the operator this pipeline's anonymizer
    /// will apply, as a `Selection` event, without touching the document.
    fn pick(&self, entities: &mut dyn EntityGroup, scope: &Scope) -> Result<()>;

    /// The pipeline as `&mut dyn Any`, to `downcast_mut` to a concrete
    /// `ModalityPipeline<M>`, how [`with_analyzer`] / [`with_anonymizer`] reach
    /// into an already-registered pipeline to replace one half.
    ///
    /// [`with_analyzer`]: crate::Orchestrator::with_analyzer
    /// [`with_anonymizer`]: crate::Orchestrator::with_anonymizer
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<M> ErasedPipeline for ModalityPipeline<M>
where
    M: Modality,
    Vec<Entity<M>>: EntityGroup,
    M::Artifact: ArtifactGroup,
{
    fn matches(&self, handle: &StreamMut<'_>) -> bool {
        handle.is::<M>()
    }

    fn analyze_streams<'a>(
        &'a self,
        handles: &'a mut [StreamMut<'_>],
        scope: &'a Scope,
        policy: HiddenContentPolicy,
        seeds: &'a [&'a dyn ArtifactGroup],
    ) -> BoxFuture<'a, Result<Vec<PartAnalysis>>> {
        Box::pin(async move {
            // The orchestrator grouped these handles by `matches`, so every one
            // downcasts; a `None` here would mean a caller mixed modalities.
            let mut typed = Vec::with_capacity(handles.len());
            for handle in handles.iter_mut() {
                let stream = handle.downcast_mut::<M>().ok_or_else(|| {
                    Error::malformed(format!("batched part is not {} despite matching", M::NAME))
                })?;
                typed.push(stream);
            }
            let mut streams: Vec<_> = typed.iter_mut().collect();
            let flagged = streams
                .iter()
                .map(|stream| flagged::entities(stream, scope, policy))
                .collect::<Result<Vec<_>>>()?;
            // A matching prior artifact restores; anything else (first-pass
            // `NoArtifact`, another modality) is `None`, so the group enriches
            // from scratch.
            let seeds = seeds
                .iter()
                .map(|seed| seed.as_any().downcast_ref::<M::Artifact>().cloned())
                .collect();
            let mut analyses = self
                .analyzer
                .analyze_streams(&mut streams, scope, seeds)
                .await?;
            for (analysis, entities) in analyses.iter_mut().zip(flagged) {
                analysis.entities.extend(entities);
            }
            Ok(analyses
                .into_iter()
                .map(|analysis| PartAnalysis {
                    entities: Box::new(analysis.entities),
                    artifact: analysis
                        .artifact
                        .map(|artifact| Box::new(artifact) as Box<dyn ArtifactGroup>),
                })
                .collect())
        })
    }

    fn apply_stream<'a>(
        &'a self,
        handle: &'a mut StreamMut<'_>,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move {
            // The redaction walk picks this pipeline from the report's modality,
            // not from the stream, and `anonymize_with` accepts a hand-built or
            // deserialized report. Such a report can key a part under one modality
            // while the stream there is another, so a mismatch is malformed input,
            // not an invariant break.
            let Some(mut stream) = handle.downcast_mut::<M>() else {
                return Err(Error::malformed(format!(
                    "report keys this part as {} but its stream is not",
                    M::NAME
                )));
            };
            self.anonymizer
                .anonymize(&mut stream, typed::<M>(entities)?, scope)
                .await
        })
    }

    fn pick(&self, entities: &mut dyn EntityGroup, scope: &Scope) -> Result<()> {
        self.anonymizer.pick(typed::<M>(entities)?, scope);
        Ok(())
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// A report part's entities as modality `M`.
///
/// # Errors
///
/// [`MalformedInput`](elide_core::ErrorKind::MalformedInput) when they are of
/// another modality.
fn typed<M: Modality>(entities: &mut dyn EntityGroup) -> Result<&mut Vec<Entity<M>>> {
    entities
        .as_any_mut()
        .downcast_mut::<Vec<Entity<M>>>()
        .ok_or_else(|| {
            Error::malformed(format!("report entities for this part are not {}", M::NAME))
        })
}
