//! [`ErasedPipeline`]: the type-erased pipeline the [`Orchestrator`] stores per
//! modality, so a document part can be offered to each pipeline until one
//! matches without the orchestrator naming the modality statically.
//!
//! [`Orchestrator`]: crate::Orchestrator

use std::any::{Any, TypeId};

use bytes::Bytes;
use elide_codec::{DocumentHandle, UntypedDocumentHandle};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;

use super::ModalityPipeline;
use super::outcome::{AnalyzeOutcome, BoxFuture, InPlaceAnalysis};
use crate::directives::AnnotationSet;
use crate::report::EntityGroup;

/// A type-erased pipeline the orchestrator stores per modality.
///
/// Every document part — the body and each container part — is an
/// [`UntypedDocumentHandle`] offered to each pipeline until one matches by
/// modality, so the orchestrator never needs to name the modality
/// statically. The phases:
/// - [`analyze`] takes an *owned* handle (a freshly-decoded container part);
///   on a modality match it detects and hands back the handle plus the boxed
///   entities, else returns the handle untouched for the next pipeline.
/// - [`analyze_in_place`] borrows a handle (the document body the caller
///   owns); on a match it detects and returns the boxed entities, else
///   `None`.
/// - [`apply_in_place`] re-drives a borrowed handle with its (possibly
///   edited) boxed entities, redacting it in place — for the body, which the
///   caller re-encodes itself.
/// - [`apply_part`] does the same on an owned handle but re-encodes to
///   redacted bytes — for a container part, spliced back into the container.
///
/// [`analyze`]: ErasedPipeline::analyze
/// [`analyze_in_place`]: ErasedPipeline::analyze_in_place
/// [`apply_in_place`]: ErasedPipeline::apply_in_place
/// [`apply_part`]: ErasedPipeline::apply_part
pub(crate) trait ErasedPipeline: Send + Sync {
    fn analyze<'a>(
        &'a self,
        handle: UntypedDocumentHandle,
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
    ) -> BoxFuture<'a, Result<AnalyzeOutcome>>;

    fn analyze_in_place<'a>(
        &'a self,
        handle: &'a mut UntypedDocumentHandle,
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
    ) -> BoxFuture<'a, Result<InPlaceAnalysis>>;

    fn apply_in_place<'a>(
        &'a self,
        handle: &'a mut UntypedDocumentHandle,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<()>>;

    fn apply_part<'a>(
        &'a self,
        handle: UntypedDocumentHandle,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<Bytes>>;

    /// The pipeline as `&mut dyn Any`, to `downcast_mut` to a concrete
    /// `ModalityPipeline<M>` — how [`with_analyzer`] / [`with_anonymizer`] reach
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
    DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
{
    fn analyze<'a>(
        &'a self,
        handle: UntypedDocumentHandle,
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
    ) -> BoxFuture<'a, Result<AnalyzeOutcome>> {
        Box::pin(async move {
            let mut handle = match handle.into::<M>() {
                Ok(handle) => handle,
                Err(returned) => return Ok(AnalyzeOutcome::Rejected(returned)),
            };
            let regions = annotations.get::<M>();
            let analysis = ModalityPipeline::analyze(self, &mut handle, scope, &regions).await?;
            Ok(AnalyzeOutcome::Accepted {
                modality: TypeId::of::<M>(),
                handle: UntypedDocumentHandle::new(handle),
                entities: Box::new(analysis.entities),
                #[cfg(feature = "usage")]
                usage: analysis.usage,
            })
        })
    }

    fn analyze_in_place<'a>(
        &'a self,
        handle: &'a mut UntypedDocumentHandle,
        scope: &'a Scope,
        annotations: &'a AnnotationSet,
    ) -> BoxFuture<'a, Result<InPlaceAnalysis>> {
        Box::pin(async move {
            let Some(typed) = handle.downcast_mut::<M>() else {
                return Ok(None); // not this pipeline's modality
            };
            let regions = annotations.get::<M>();
            let analysis = ModalityPipeline::analyze(self, typed, scope, &regions).await?;
            let entities = Box::new(analysis.entities) as Box<dyn EntityGroup>;
            #[cfg(feature = "usage")]
            return Ok(Some((entities, analysis.usage)));
            #[cfg(not(feature = "usage"))]
            Ok(Some(entities))
        })
    }

    fn apply_in_place<'a>(
        &'a self,
        handle: &'a mut UntypedDocumentHandle,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // The handle and entities were matched to this pipeline's `M` by
            // the orchestrator (stored modality `TypeId`), so both downcasts
            // hold. Take the typed handle out, redact, and put it back.
            let mut typed = handle
                .take::<M>()
                .unwrap_or_else(|| unreachable!("apply_in_place handle modality mismatch"));
            let entities = entities
                .as_any_mut()
                .downcast_mut::<Vec<Entity<M>>>()
                .expect("apply_in_place entities modality mismatch");
            self.apply(&mut typed, entities, scope).await?;
            *handle = UntypedDocumentHandle::new(typed);
            Ok(())
        })
    }

    fn apply_part<'a>(
        &'a self,
        handle: UntypedDocumentHandle,
        entities: &'a mut dyn EntityGroup,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<Bytes>> {
        Box::pin(async move {
            // The handle and entities were matched to this pipeline's `M`, so
            // both downcasts hold.
            let mut handle = handle
                .into::<M>()
                .unwrap_or_else(|_| unreachable!("apply_part handle modality mismatch"));
            let entities = entities
                .as_any_mut()
                .downcast_mut::<Vec<Entity<M>>>()
                .expect("apply_part entities modality mismatch");
            self.apply(&mut handle, entities, scope).await?;
            Ok(handle.encode()?.to_bytes())
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
