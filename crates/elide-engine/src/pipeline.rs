//! Per-modality pipeline and its type-erased form, used by the
//! [`Orchestrator`] to drive a document's body and
//! its container parts across two phases (analyze, then apply).
//!
//! [`Orchestrator`]: super::Orchestrator

use std::any::TypeId;
use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use elide_codec::{DocumentHandle, UntypedDocumentHandle};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;
#[cfg(feature = "usage")]
use elide_core::recognition::Usage;
use elide_core::recognition::annotation::Annotations;
use elide_detection::{Analysis, Analyzer};
use elide_redaction::Anonymizer;

use super::directives::AnnotationSet;
use super::report::EntityGroup;

/// The concrete analyze + redact pipeline for one modality `M`.
///
/// The [`Scope`] and region [`Annotations`] are supplied per analysis (via
/// [`Directives`]) as arguments to [`analyze`].
///
/// [`Annotations`]: elide_core::recognition::annotation::Annotations
/// [`Directives`]: super::Directives
/// [`analyze`]: Self::analyze
pub(super) struct ModalityPipeline<M: Modality> {
    pub(super) analyzer: Analyzer<M>,
    pub(super) anonymizer: Anonymizer<M>,
}

impl<M> ModalityPipeline<M>
where
    M: Modality,
    DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
{
    /// Detect the entities in `handle` (in source coordinates), without
    /// redacting. The caller may edit the returned set before applying.
    pub(super) async fn analyze(
        &self,
        handle: &mut DocumentHandle<M>,
        scope: &Scope,
        annotations: &Annotations<M>,
    ) -> Result<Analysis<M>> {
        self.analyzer
            .analyze_stream_with(handle, scope, annotations)
            .await
    }

    /// Apply `entities` to `handle` in place: the redactions land in the
    /// handle, ready for its eventual `encode`. `scope` is passed to selection
    /// so scope-aware rules can branch on request context.
    pub(super) async fn apply(
        &self,
        handle: &mut DocumentHandle<M>,
        entities: &mut [Entity<M>],
        scope: &Scope,
    ) -> Result<()> {
        self.anonymizer.anonymize(handle, entities, scope).await
    }
}

/// A boxed, pinned, `Send` future — the erased async return shape.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The boxed entities a matched in-place analysis produced; `None` when the
/// pipeline's modality did not match the handle. Under the `usage` feature it
/// also carries the per-component [`Usage`] the analysis recorded. The result
/// of [`ErasedPipeline::analyze_in_place`].
#[cfg(feature = "usage")]
type InPlaceAnalysis = Option<(Box<dyn EntityGroup>, Vec<Usage>)>;
/// The boxed entities a matched in-place analysis produced; `None` when the
/// pipeline's modality did not match the handle. The result of
/// [`ErasedPipeline::analyze_in_place`].
#[cfg(not(feature = "usage"))]
type InPlaceAnalysis = Option<Box<dyn EntityGroup>>;

/// The result of offering a decoded handle to a pipeline for analysis: the
/// pipeline either accepts it (its modality matched) and returns the
/// detected entities boxed by modality, or rejects it (a different
/// modality) and hands the handle back for another pipeline to try.
pub(super) enum AnalyzeOutcome {
    /// Modality matched: the matched modality's `TypeId`, the retained
    /// handle, its boxed `Vec<Entity<M>>` (recoverable as that modality), and
    /// the per-component [`Usage`] the analysis recorded.
    Accepted {
        modality: TypeId,
        handle: UntypedDocumentHandle,
        entities: Box<dyn EntityGroup>,
        #[cfg(feature = "usage")]
        usage: Vec<Usage>,
    },
    /// Not this pipeline's modality; the undecoded handle is returned.
    Rejected(UntypedDocumentHandle),
}

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
pub(super) trait ErasedPipeline: Send + Sync {
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
}
