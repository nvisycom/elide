//! Per-modality pipeline and its type-erased form, used by the
//! [`Orchestrator`] to drive a document's body and
//! its container parts across two phases (analyze, then apply).
//!
//! [`Orchestrator`]: super::Orchestrator

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use elide_codec::{DocumentHandle, UntypedDocumentHandle};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;
use elide_detection::Analyzer;
use elide_redaction::Anonymizer;

use super::report::EntityGroup;

/// The concrete analyze + redact pipeline for one modality `M`.
pub(super) struct ModalityPipeline<M: Modality> {
    pub(super) analyzer: Analyzer<M>,
    pub(super) anonymizer: Anonymizer<M>,
    pub(super) scope: Scope<M>,
}

impl<M> ModalityPipeline<M>
where
    M: Modality,
    DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
{
    /// Detect the entities in `handle` (in source coordinates), without
    /// redacting. The caller may edit the returned set before applying.
    pub(super) async fn analyze(&self, handle: &mut DocumentHandle<M>) -> Result<Vec<Entity<M>>> {
        self.analyzer.analyze_stream(handle, &self.scope).await
    }

    /// Apply `entities` to `handle` in place: the redactions land in the
    /// handle, ready for its eventual `encode`.
    pub(super) async fn apply(
        &self,
        handle: &mut DocumentHandle<M>,
        entities: &mut [Entity<M>],
    ) -> Result<()> {
        self.anonymizer.anonymize(handle, entities).await
    }
}

/// A boxed, pinned, `Send` future — the erased async return shape.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The result of offering a decoded handle to a pipeline for analysis: the
/// pipeline either accepts it (its modality matched) and returns the
/// detected entities boxed by modality, or rejects it (a different
/// modality) and hands the handle back for another pipeline to try.
pub(super) enum AnalyzeOutcome {
    /// Modality matched: the matched modality's `TypeId`, the retained
    /// handle, and its boxed `Vec<Entity<M>>` (recoverable as that
    /// modality).
    Accepted {
        modality: std::any::TypeId,
        handle: UntypedDocumentHandle,
        entities: Box<dyn EntityGroup>,
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
    fn analyze(&self, handle: UntypedDocumentHandle) -> BoxFuture<'_, Result<AnalyzeOutcome>>;

    fn analyze_in_place<'a>(
        &'a self,
        handle: &'a mut UntypedDocumentHandle,
    ) -> BoxFuture<'a, Result<Option<Box<dyn EntityGroup>>>>;

    fn apply_in_place<'a>(
        &'a self,
        handle: &'a mut UntypedDocumentHandle,
        entities: &'a mut dyn EntityGroup,
    ) -> BoxFuture<'a, Result<()>>;

    fn apply_part<'a>(
        &'a self,
        handle: UntypedDocumentHandle,
        entities: &'a mut dyn EntityGroup,
    ) -> BoxFuture<'a, Result<Bytes>>;
}

impl<M> ErasedPipeline for ModalityPipeline<M>
where
    M: Modality,
    Vec<Entity<M>>: EntityGroup,
    DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
{
    fn analyze(&self, handle: UntypedDocumentHandle) -> BoxFuture<'_, Result<AnalyzeOutcome>> {
        Box::pin(async move {
            let mut handle = match handle.into::<M>() {
                Ok(handle) => handle,
                Err(returned) => return Ok(AnalyzeOutcome::Rejected(returned)),
            };
            let entities = ModalityPipeline::analyze(self, &mut handle).await?;
            Ok(AnalyzeOutcome::Accepted {
                modality: std::any::TypeId::of::<M>(),
                handle: UntypedDocumentHandle::new(handle),
                entities: Box::new(entities),
            })
        })
    }

    fn analyze_in_place<'a>(
        &'a self,
        handle: &'a mut UntypedDocumentHandle,
    ) -> BoxFuture<'a, Result<Option<Box<dyn EntityGroup>>>> {
        Box::pin(async move {
            let Some(typed) = handle.downcast_mut::<M>() else {
                return Ok(None); // not this pipeline's modality
            };
            let entities = ModalityPipeline::analyze(self, typed).await?;
            Ok(Some(Box::new(entities) as Box<dyn EntityGroup>))
        })
    }

    fn apply_in_place<'a>(
        &'a self,
        handle: &'a mut UntypedDocumentHandle,
        entities: &'a mut dyn EntityGroup,
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
            self.apply(&mut typed, entities).await?;
            *handle = UntypedDocumentHandle::new(typed);
            Ok(())
        })
    }

    fn apply_part<'a>(
        &'a self,
        handle: UntypedDocumentHandle,
        entities: &'a mut dyn EntityGroup,
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
            self.apply(&mut handle, entities).await?;
            Ok(handle.encode()?.to_bytes())
        })
    }
}
