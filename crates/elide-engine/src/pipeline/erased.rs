//! [`ErasedPipeline`]: the type-erased pipeline the [`Orchestrator`] stores per
//! modality, so a document's stream part can be offered to each pipeline until
//! one matches without the orchestrator naming the modality statically.
//!
//! [`Orchestrator`]: crate::Orchestrator

use std::any::Any;

use elide_codec::{ErasedStream, TypedStream};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;

use super::ModalityPipeline;
use super::outcome::{BoxFuture, InPlaceAnalysis};
use crate::analysis::{ArtifactGroup, EntityGroup};
use crate::directives::AnnotationSet;

/// A type-erased pipeline the orchestrator stores per modality.
///
/// Every document stream part is an [`ErasedStream`] offered to each pipeline
/// until one matches by modality, so the orchestrator never needs to name the
/// modality statically. The analyze phase is seeded with the group's prior
/// enrichment artifact, `NoArtifact` on a first pass, a restored artifact on a
/// re-run, so the same path serves both. The phases:
/// - [`analyze_stream`] borrows a stream in place; on a modality match it
///   detects and returns the boxed entities and artifact, else `None`.
/// - [`apply_stream`] re-drives a matched stream with its (possibly edited)
///   boxed entities, redacting it in place; the document re-encodes itself.
///
/// [`analyze_stream`]: ErasedPipeline::analyze_stream
/// [`apply_stream`]: ErasedPipeline::apply_stream
pub(crate) trait ErasedPipeline: Send + Sync {
    /// Analyze a stream part in place, seeded with the group's prior enrichment
    /// `artifact`, `NoArtifact` on a first pass, so it enriches from scratch; a
    /// restored artifact on a re-run, so it re-recognizes without re-enriching.
    /// On a modality match it detects and returns the boxed entities and
    /// artifact; `None` when the stream's modality is not this pipeline's.
    fn analyze_stream<'a>(
        &'a self,
        handle: &'a mut ErasedStream,
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
        artifact: &'a dyn ArtifactGroup,
    ) -> BoxFuture<'a, Result<InPlaceAnalysis>>;

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
    fn analyze_stream<'a>(
        &'a self,
        handle: &'a mut ErasedStream,
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
        artifact: &'a dyn ArtifactGroup,
    ) -> BoxFuture<'a, Result<InPlaceAnalysis>> {
        Box::pin(async move {
            let Some(stream) = handle.downcast_mut::<M>() else {
                return Ok(None); // not this pipeline's modality
            };
            let regions = annotations.get::<M>();
            // A prior artifact for this modality restores as `Some` (even when
            // empty, that is a valid enrichment result); a `NoArtifact`
            // first-pass seed or a mismatched-modality seed downcasts to `None`,
            // so the group enriches from scratch.
            let seed = artifact.as_any().downcast_ref::<M::Artifact>().cloned();
            let analysis = self
                .analyzer
                .analyze_stream_in(stream, scope, &regions, seed)
                .await?;
            let entities = Box::new(analysis.entities) as Box<dyn EntityGroup>;
            let artifact = analysis
                .artifact
                .map(|a| Box::new(a) as Box<dyn ArtifactGroup>);
            #[cfg(feature = "usage")]
            return Ok(Some((entities, artifact, analysis.usage)));
            #[cfg(not(feature = "usage"))]
            Ok(Some((entities, artifact)))
        })
    }

    fn apply_stream<'a>(
        &'a self,
        handle: &'a mut ErasedStream,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // The stream and entities were matched to this pipeline's `M` by the
            // orchestrator (stored modality `TypeId`), so both downcasts hold.
            let stream = handle
                .downcast_mut::<M>()
                .unwrap_or_else(|| unreachable!("apply_stream modality mismatch"));
            let entities = entities
                .as_any_mut()
                .downcast_mut::<Vec<Entity<M>>>()
                .expect("apply_stream entities modality mismatch");
            self.anonymizer.anonymize(stream, entities, scope).await
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
