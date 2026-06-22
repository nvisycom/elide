//! Per-modality pipeline and its type-erased form, used by the
//! [`Orchestrator`](super::Orchestrator) to drive a document's body and
//! its container parts across two phases (analyze, then apply).

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::recognition::Scope;

use super::report::EntityGroup;
use crate::codec::{DocumentHandle, UntypedDocumentHandle};
use crate::{Analyzer, Anonymizer};

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
        entities: &[Entity<M>],
    ) -> Result<()> {
        self.anonymizer.anonymize(handle, entities).await
    }
}

/// A boxed, pinned, `Send` future — the erased async return shape.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The result of offering a decoded part to a pipeline for analysis: the
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
/// The split phases:
/// - [`analyze_part`] downcasts a decoded part to its modality `M`; on a
///   match it detects entities and hands back the retained handle plus the
///   boxed entities, else returns the handle untouched.
/// - [`apply_part`] re-drives a retained part handle with its (possibly
///   edited) boxed entities and re-encodes it to redacted bytes.
/// - [`as_any`] recovers the concrete pipeline so the orchestrator can
///   drive a document's own-modality body, whose `M` it knows statically.
///
/// [`analyze_part`]: ErasedPipeline::analyze_part
/// [`apply_part`]: ErasedPipeline::apply_part
/// [`as_any`]: ErasedPipeline::as_any
pub(super) trait ErasedPipeline: Send + Sync {
    fn analyze_part(&self, part: UntypedDocumentHandle) -> BoxFuture<'_, Result<AnalyzeOutcome>>;

    fn apply_part<'a>(
        &'a self,
        handle: UntypedDocumentHandle,
        entities: &'a dyn EntityGroup,
    ) -> BoxFuture<'a, Result<Bytes>>;

    fn as_any(&self) -> &dyn Any;
}

impl<M> ErasedPipeline for ModalityPipeline<M>
where
    M: Modality,
    Vec<Entity<M>>: EntityGroup,
    DocumentHandle<M>: StreamDataReader<M> + DataReader<M> + DataWriter<M>,
{
    fn analyze_part(&self, part: UntypedDocumentHandle) -> BoxFuture<'_, Result<AnalyzeOutcome>> {
        Box::pin(async move {
            let mut handle = match part.into::<M>() {
                Ok(handle) => handle,
                Err(returned) => return Ok(AnalyzeOutcome::Rejected(returned)),
            };
            let entities = self.analyze(&mut handle).await?;
            Ok(AnalyzeOutcome::Accepted {
                modality: std::any::TypeId::of::<M>(),
                handle: UntypedDocumentHandle::new(handle),
                entities: Box::new(entities),
            })
        })
    }

    fn apply_part<'a>(
        &'a self,
        handle: UntypedDocumentHandle,
        entities: &'a dyn EntityGroup,
    ) -> BoxFuture<'a, Result<Bytes>> {
        Box::pin(async move {
            // The handle and entities were produced by this same pipeline
            // in `analyze_part`, so both downcasts hold.
            let mut handle = handle
                .into::<M>()
                .unwrap_or_else(|_| unreachable!("apply_part handle modality mismatch"));
            let entities = entities
                .as_any()
                .downcast_ref::<Vec<Entity<M>>>()
                .expect("apply_part entities modality mismatch");
            self.apply(&mut handle, entities).await?;
            Ok(handle.encode()?.to_bytes())
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
