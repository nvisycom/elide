//! [`DocumentSelections`]: the operator picks for a whole document — the
//! body and every container part — as returned by [`Orchestrator::select`].
//!
//! [`Orchestrator::select`]: crate::Orchestrator::select

use std::collections::HashMap;

use elide_codec::PartId;
use elide_redaction::SelectionView;

use super::group::SelectionGroup;

/// The operator picks for a whole document, mirroring the shape of a
/// [`Report`]: the body's picks (if a body pipeline ran) and each container
/// part's, keyed by [`PartId`].
///
/// Returned by [`Orchestrator::select`] — the reviewable decision phase run
/// over a report's detected entities, reading no document data. Each group is
/// type-erased; downcast a group to `Vec<Selection<M>>` for the in-process
/// apply path, or take [`views`](Self::views) for the serializable,
/// modality-free projection a review layer ships and displays.
///
/// [`Report`]: super::Report
/// [`Orchestrator::select`]: crate::Orchestrator::select
/// [`Selection<M>`]: elide_redaction::Selection
#[derive(Default)]
pub struct DocumentSelections {
    /// The body's picks, if a body pipeline is registered and the report has
    /// a body. A document has exactly one body, so at most one group.
    pub body: Option<Box<dyn SelectionGroup>>,
    /// Each container part's picks, keyed by [`PartId`]. A part appears only
    /// when a pipeline is registered for its modality.
    pub parts: HashMap<PartId, Box<dyn SelectionGroup>>,
}

impl DocumentSelections {
    /// The serializable, modality-free projection of every pick in the
    /// document — the body's views first, then each part's — for display or
    /// wire transport.
    ///
    /// Works through erasure: no modality is named. To attribute a view to its
    /// part, project each part group with [`SelectionGroup::views`] directly.
    pub fn views(&self) -> Vec<SelectionView> {
        let body = self.body.iter().flat_map(|g| g.views());
        let parts = self.parts.values().flat_map(|g| g.views());
        body.chain(parts).collect()
    }
}
