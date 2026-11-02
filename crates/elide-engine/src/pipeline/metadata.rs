//! [`MetadataPipeline`]: the built-in pipeline every orchestrator drives a
//! document's metadata through.

use std::any::Any;

use elide_codec::ErasedStream;
use elide_core::entity::Entity;
use elide_core::modality::metadata::Metadata;
use elide_core::modality::{ModalityLocation, StreamDataReader};
use elide_core::recognition::Scope;
use elide_core::{Error, Result};
use elide_detection::Analyzer;
use elide_operator::operators::Erase;
use elide_redaction::{Anonymizer, Rule};

use super::ModalityPipeline;
use super::erased::ErasedPipeline;
use super::outcome::{BoxFuture, PartAnalysis};
use crate::analysis::{ArtifactGroup, EntityGroup};
use crate::directives::AnnotationSet;

/// The pipeline for the [`Metadata`] modality, present in every orchestrator.
///
/// A metadata field is detected by its format's schema: the codec labels each
/// field it surfaces, so every field is an entity carrying that label, kept
/// when the scope's catalog declares it and no exclusion covers it. Nothing is
/// registered for this; a caller's [`Analyzer<Metadata>`] adds entities on top
/// of the fields, and a caller's [`Anonymizer<Metadata>`] decides how the
/// selected fields are treated. The default anonymizer erases every one.
///
/// [`with_analyzer`] and [`with_anonymizer`] reach the wrapped pipeline
/// through [`as_any_mut`](ErasedPipeline::as_any_mut), so neither can remove
/// the schema detection.
///
/// [`with_analyzer`]: crate::Orchestrator::with_analyzer
/// [`with_anonymizer`]: crate::Orchestrator::with_anonymizer
pub(crate) struct MetadataPipeline {
    inner: ModalityPipeline<Metadata>,
}

impl Default for MetadataPipeline {
    fn default() -> Self {
        Self {
            inner: ModalityPipeline::new(
                Analyzer::new(),
                Anonymizer::new().with(Rule::fallback(Erase)),
            ),
        }
    }
}

impl MetadataPipeline {
    /// One entity per field `stream` surfaces whose label the scope's catalog
    /// declares and no exclusion covers.
    fn fields(
        stream: &mut ErasedStream,
        scope: &Scope,
        annotations: &AnnotationSet,
    ) -> Result<Vec<Entity<Metadata>>> {
        let stream = stream
            .downcast_mut::<Metadata>()
            .ok_or_else(|| Error::malformed("batched part is not metadata despite matching"))?;
        let exclusions = annotations.get::<Metadata>().exclusions;
        let entities = stream
            .chunks()?
            .iter()
            .filter_map(|chunk| {
                Metadata::field_entity(chunk.data.key(), chunk.data.label().clone())
            })
            .filter(|entity| {
                !exclusions
                    .iter()
                    .any(|exclusion| entity.location.overlaps(&exclusion.location))
            })
            .collect();
        Ok(scope.catalog.retain_declared(entities))
    }
}

impl ErasedPipeline for MetadataPipeline {
    fn matches(&self, handle: &ErasedStream) -> bool {
        self.inner.matches(handle)
    }

    fn analyze_streams<'a>(
        &'a self,
        handles: &'a mut [&'a mut ErasedStream],
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
        seeds: &'a [&'a dyn ArtifactGroup],
    ) -> BoxFuture<'a, Result<Vec<PartAnalysis>>> {
        Box::pin(async move {
            let mut fields = Vec::with_capacity(handles.len());
            for handle in handles.iter_mut() {
                fields.push(Self::fields(handle, scope, annotations)?);
            }
            let mut analyses = self
                .inner
                .analyze_streams(handles, scope, annotations, seeds)
                .await?;
            for ((entities, _), fields) in analyses.iter_mut().zip(fields) {
                if let Some(entities) = entities
                    .as_any_mut()
                    .downcast_mut::<Vec<Entity<Metadata>>>()
                {
                    entities.extend(fields);
                }
            }
            Ok(analyses)
        })
    }

    fn apply_stream<'a>(
        &'a self,
        handle: &'a mut ErasedStream,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<()>> {
        self.inner.apply_stream(handle, entities, scope)
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut self.inner
    }
}
