//! The [`Report`]'s per-group entries: [`BodyReport`] for the document body
//! and [`PartReport`] for each container part.
//!
//! [`Report`]: super::Report

use std::any::TypeId;

use elide_codec::UntypedDocumentHandle;

use super::EntityGroup;

/// The document body's entry: its detected entities and the modality they
/// belong to.
///
/// A document has exactly one body, so a [`Report`] holds at most one of
/// these. The `modality` is the routing key — the pipeline registered for it
/// analyzes and applies the body.
///
/// [`Report`]: super::Report
pub(crate) struct BodyReport {
    /// The body's modality, keying the pipeline that handles it.
    pub(crate) modality: TypeId,
    /// The body's detected entities (a `Vec<Entity<M>>`).
    pub(crate) entities: Box<dyn EntityGroup>,
}

/// One container part captured during analysis: its detected entities, the
/// modality they belong to, and — for the same-process fast path — the
/// decoded part handle.
pub(crate) struct PartReport {
    /// The part's modality, the routing key for [`anonymize_with`]: it
    /// re-fetches the part from the container and applies through the
    /// pipeline registered for this modality.
    ///
    /// [`anonymize_with`]: crate::Orchestrator::anonymize_with
    pub(crate) modality: TypeId,
    /// The decoded part handle, retained from analysis as a same-process
    /// cache. `Some` after [`analyze`] (so apply re-drives it directly with
    /// no second decode); `None` for a [`Report`] built by hand or rebuilt
    /// from serialized entities, where apply re-decodes the part from the
    /// container instead.
    ///
    /// Never serialized — a live decoded document is not data.
    ///
    /// [`analyze`]: crate::Orchestrator::analyze
    /// [`Report`]: super::Report
    pub(crate) handle: Option<UntypedDocumentHandle>,
    /// The part's detected entities (a `Vec<Entity<P>>`).
    pub(crate) entities: Box<dyn EntityGroup>,
}
