//! [`ErasedPipeline`]: the type-erased pipeline the [`Orchestrator`] stores per
//! modality, so a document's stream part can be offered to each pipeline until
//! one matches without the orchestrator naming the modality statically.
//!
//! [`Orchestrator`]: crate::Orchestrator

use std::any::Any;

use elide_codec::{ErasedStream, TypedStream};
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;
use elide_core::{Error, ErrorKind, Result};

use super::ModalityPipeline;
use super::outcome::{BoxFuture, PartAnalysis};
use crate::analysis::{ArtifactGroup, EntityGroup};
use crate::directives::AnnotationSet;

/// A type-erased pipeline the orchestrator stores per modality.
///
/// Every document stream part is an [`ErasedStream`] the orchestrator buckets by
/// modality ([`matches`]) so it never names the modality statically. The analyze
/// phase is seeded with the group's prior enrichment artifact, `NoArtifact` on a
/// first pass, a restored artifact on a re-run, so the same path serves both. The
/// phases:
/// - [`analyze_streams`] analyzes the matched parts together, coalescing
///   enrichment into one provider round-trip, and returns one result per handle.
/// - [`apply_stream`] re-drives a matched stream with its (possibly edited)
///   boxed entities, redacting it in place; the document re-encodes itself.
///
/// [`matches`]: ErasedPipeline::matches
/// [`analyze_streams`]: ErasedPipeline::analyze_streams
/// [`apply_stream`]: ErasedPipeline::apply_stream
pub(crate) trait ErasedPipeline: Send + Sync {
    /// Whether `handle` carries this pipeline's modality — a non-mutating probe
    /// the orchestrator uses to bucket parts by modality before dispatching a
    /// batch, without a destructive try-each-pipeline downcast.
    fn matches(&self, handle: &ErasedStream) -> bool;

    /// Analyze several stream parts of this pipeline's modality together,
    /// coalescing enrichment into one batched provider round-trip, and return one
    /// [`PartAnalysis`] per handle in order. Each handle is paired with its prior
    /// enrichment `seed` (`NoArtifact` on a first pass). Every handle must already
    /// match this pipeline's modality (the orchestrator groups them with
    /// [`matches`](Self::matches) first); a non-matching handle is a caller error.
    fn analyze_streams<'a>(
        &'a self,
        handles: &'a mut [&'a mut ErasedStream],
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
        seeds: &'a [&'a dyn ArtifactGroup],
    ) -> BoxFuture<'a, Result<Vec<PartAnalysis>>>;

    /// Redact a matched stream part in place with its (possibly edited) boxed
    /// entities; the document re-encodes the stream itself on `encode`.
    fn apply_stream<'a>(
        &'a self,
        handle: &'a mut ErasedStream,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<()>>;

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
    TypedStream<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
{
    fn matches(&self, handle: &ErasedStream) -> bool {
        handle.is::<M>()
    }

    fn analyze_streams<'a>(
        &'a self,
        handles: &'a mut [&'a mut ErasedStream],
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
        seeds: &'a [&'a dyn ArtifactGroup],
    ) -> BoxFuture<'a, Result<Vec<PartAnalysis>>> {
        Box::pin(async move {
            let regions = annotations.get::<M>();
            // The orchestrator grouped these handles by `matches`, so every one
            // downcasts; a `None` here would mean a caller mixed modalities.
            let mut streams = Vec::with_capacity(handles.len());
            for handle in handles.iter_mut() {
                let stream = handle.downcast_mut::<M>().ok_or_else(|| {
                    Error::new(
                        ErrorKind::MalformedInput,
                        format!("batched part is not {} despite matching", M::NAME),
                    )
                })?;
                streams.push(stream);
            }
            // Each seed downcasts as in `analyze_stream`: a matching prior artifact
            // restores, anything else (first-pass `NoArtifact`, other modality) is
            // `None` so the group enriches from scratch.
            let seeds = seeds
                .iter()
                .map(|seed| seed.as_any().downcast_ref::<M::Artifact>().cloned())
                .collect();
            let analyses = self
                .analyzer
                .analyze_streams(&mut streams, scope, &regions, seeds)
                .await?;
            Ok(analyses
                .into_iter()
                .map(|analysis| {
                    let entities = Box::new(analysis.entities) as Box<dyn EntityGroup>;
                    let artifact = analysis
                        .artifact
                        .map(|a| Box::new(a) as Box<dyn ArtifactGroup>);
                    (entities, artifact)
                })
                .collect())
        })
    }

    fn apply_stream<'a>(
        &'a self,
        handle: &'a mut ErasedStream,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // `apply_parts` picks this pipeline from the report's stored modality,
            // not from the stream, and `anonymize_with` accepts a hand-built or
            // deserialized report. Such a report can key a part under one modality
            // while the stream there is another, so a mismatch is malformed input,
            // not an invariant break.
            let Some(stream) = handle.downcast_mut::<M>() else {
                return Err(Error::new(
                    ErrorKind::MalformedInput,
                    format!("report keys this part as {} but its stream is not", M::NAME),
                ));
            };
            let Some(entities) = entities.as_any_mut().downcast_mut::<Vec<Entity<M>>>() else {
                return Err(Error::new(
                    ErrorKind::MalformedInput,
                    format!("report entities for this part are not {}", M::NAME),
                ));
            };
            self.anonymizer.anonymize(stream, entities, scope).await
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
