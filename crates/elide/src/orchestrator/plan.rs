//! [`DocumentPlan`]: the detected-but-not-yet-applied entities of a whole
//! document, editable per modality before [`apply`](super::Orchestrator).
//!
//! Detection (`analyze_document`) and redaction (`apply`) are split so a
//! caller can inspect and edit the entities in between — drop a
//! false-positive, retag, retarget a span. A document's entities span
//! several coordinate systems (the body's modality, plus each container
//! part's), so the plan keeps them separated: the body entities keyed by
//! their modality, and each part's entities keyed by the part id, each
//! editable through a typed accessor.
//!
//! The entities are recovered as concrete `Vec<Entity<M>>` through the
//! typed accessors, so a caller that wants to serialize them for an
//! external consumer (a review UI) can: pull each modality's slice and
//! hand it to `serde` itself, using [`part_ids`](DocumentPlan::part_ids)
//! to enumerate the parts. The part id then labels which part each group
//! belongs to.

use std::any::{Any, TypeId};
use std::collections::HashMap;

use elide_core::entity::Entity;
use elide_core::modality::Modality;

use crate::codec::UntypedDocumentHandle;

/// One container part captured during analysis: its decoded handle (kept
/// alive so `apply` can re-drive it) and its detected entities (boxed
/// `Vec<Entity<P>>`, recovered by the part's modality).
pub(super) struct PartPlan {
    /// The part's modality, for dispatching `apply` to the right pipeline.
    pub(super) modality: TypeId,
    /// The decoded part handle, retained for the apply phase.
    pub(super) handle: UntypedDocumentHandle,
    /// The part's detected entities, as `Box<Vec<Entity<P>>>`.
    pub(super) entities: Box<dyn Any + Send + Sync>,
}

/// The detected entities of a whole document, editable before apply.
///
/// Returned by [`analyze_document`] and consumed by [`apply`]. Edit the
/// body entities of modality `M` with [`entities`], and a part's with
/// [`part_entities`]; both hand back a `&mut Vec<Entity<_>>` you can
/// filter, retag, or extend before applying.
///
/// [`analyze_document`]: super::Orchestrator::analyze_document
/// [`apply`]: super::Orchestrator::apply
/// [`entities`]: DocumentPlan::entities
/// [`part_entities`]: DocumentPlan::part_entities
pub struct DocumentPlan {
    /// The body's entities, as `Box<Vec<Entity<M>>>` keyed by `M`'s
    /// `TypeId`. A document has exactly one body modality, so this holds
    /// at most one entry.
    pub(super) body: Option<(TypeId, Box<dyn Any + Send + Sync>)>,
    /// Each container part's plan, keyed by part id (a zip entry name).
    pub(super) parts: HashMap<String, PartPlan>,
}

impl DocumentPlan {
    pub(super) fn new() -> Self {
        Self {
            body: None,
            parts: HashMap::new(),
        }
    }

    /// The body entities of modality `M`, for inspection or editing.
    /// Returns `None` if the document's body is a different modality (or
    /// no body pipeline ran).
    pub fn entities<M: Modality>(&mut self) -> Option<&mut Vec<Entity<M>>> {
        let (type_id, boxed) = self.body.as_mut()?;
        if *type_id != TypeId::of::<M>() {
            return None;
        }
        boxed.downcast_mut::<Vec<Entity<M>>>()
    }

    /// The entities of the container part named `id`, as modality `P`, for
    /// inspection or editing. Returns `None` for an unknown part or a
    /// modality mismatch.
    pub fn part_entities<P: Modality>(&mut self, id: &str) -> Option<&mut Vec<Entity<P>>> {
        let part = self.parts.get_mut(id)?;
        if part.modality != TypeId::of::<P>() {
            return None;
        }
        part.entities.downcast_mut::<Vec<Entity<P>>>()
    }

    /// The ids of the container parts the plan carries, paired with each
    /// part's modality `TypeId` — for a caller enumerating what's editable.
    pub fn part_ids(&self) -> impl Iterator<Item = (&str, TypeId)> {
        self.parts.iter().map(|(id, p)| (id.as_str(), p.modality))
    }
}
