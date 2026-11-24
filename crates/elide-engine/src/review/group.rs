//! [`EditGroup`]: one part's edits, a `Vec<Edit<M>>` behind one type, each
//! checked against a policy and applied in turn.

use elide_core::entity::Entity;
use elide_core::entity::audit::Manual;
use elide_core::modality::{Modality, ModalityLocation, Overlap};
use elide_core::{Error, Result};
use uuid::Uuid;

use super::edit::{Change, Edit};
use super::policy::{Narrowed, ProposedEdit, ReviewPolicy};
use crate::PartId;
use crate::analysis::{EntityGroup, ErasedGroup, GroupItem};

/// A group of one modality's review edits, a `Vec<Edit<M>>` behind one type,
/// held erased like a report's entities.
pub(crate) trait EditGroup: ErasedGroup {
    /// Each edit's fields, in order, for the review's digest.
    fn fields(&self, out: &mut Vec<Vec<u8>>);
    /// The ids the group's includes give their entities.
    fn included(&self) -> Vec<Uuid>;
    /// An empty entity group of this modality, for a part the report does not
    /// hold yet, which an include creates.
    fn empty_entities(&self) -> Box<dyn EntityGroup>;
    /// The part's `entities` with every edit applied in order, each checked
    /// against `policy` as the edits before it left them. `entities` is left
    /// as it is, so a refused edit changes nothing.
    fn apply(
        &self,
        part: &PartId,
        entities: &dyn EntityGroup,
        policy: &dyn ReviewPolicy,
    ) -> Result<Box<dyn EntityGroup>>;
}

impl<M: Modality> GroupItem for Edit<M> {
    type Modality = M;
}

impl<M: Modality> EditGroup for Vec<Edit<M>>
where
    M::Location: serde::Serialize + serde::de::DeserializeOwned,
{
    fn fields(&self, out: &mut Vec<Vec<u8>>) {
        for edit in self {
            edit.fields(out);
        }
    }

    fn included(&self) -> Vec<Uuid> {
        self.iter()
            .filter_map(|edit| match edit.change {
                Change::Include { entity, .. } => Some(entity),
                _ => None,
            })
            .collect()
    }

    fn empty_entities(&self) -> Box<dyn EntityGroup> {
        Box::new(Vec::<Entity<M>>::new())
    }

    fn apply(
        &self,
        part: &PartId,
        entities: &dyn EntityGroup,
        policy: &dyn ReviewPolicy,
    ) -> Result<Box<dyn EntityGroup>> {
        let mut entities = entities
            .as_any()
            .downcast_ref::<Vec<Entity<M>>>()
            .ok_or_else(|| {
                Error::not_found(format!(
                    "the review edits part `{part}` as {}, which the report holds as another modality",
                    M::NAME
                ))
            })?
            .clone();
        for edit in self {
            step(part, &mut entities, edit, policy)?;
        }
        Ok(Box::new(entities))
    }
}

/// Check one `edit` of `part` against `policy`, and apply it to the part's
/// `entities` once accepted.
fn step<M: Modality>(
    part: &PartId,
    entities: &mut Vec<Entity<M>>,
    edit: &Edit<M>,
    policy: &dyn ReviewPolicy,
) -> Result<()> {
    let targets = targets(part, entities, &edit.change)?;
    let widens = match &edit.change {
        Change::Include { .. } => true,
        Change::Relocate { location, .. } => targets
            .iter()
            .all(|&i| covers(location, entities[i].location())),
        _ => false,
    };
    let narrowed: Vec<Narrowed> = if widens {
        Vec::new()
    } else {
        targets
            .iter()
            .map(|&i| Narrowed {
                id: entities[i].id(),
                label: entities[i].label().clone(),
            })
            .collect()
    };
    let decision = edit.change.intent();
    policy.check(&ProposedEdit {
        part,
        decision: &decision,
        narrowed: &narrowed,
        by: edit.by.as_deref(),
        because: edit.because.as_ref(),
    })?;

    let manual = |location: M::Location| Manual {
        intent: decision.clone(),
        location,
        attribution: edit.because.clone(),
    };
    let by = edit.by.as_deref();
    match &edit.change {
        Change::Include {
            entity: id,
            label,
            location,
        } => {
            entities.push(Entity::included(
                *id,
                label.clone(),
                location.clone(),
                edit.because.clone(),
                by,
            ));
        }
        change => {
            for i in targets {
                let at = match change {
                    Change::SuppressWithin { region } => region.clone(),
                    Change::Relocate { location, .. } => location.clone(),
                    _ => entities[i].location().clone(),
                };
                entities[i].record_manual(manual(at), by);
            }
        }
    }
    Ok(())
}

/// The indices of the entities of `part` that `change` touches: none for an
/// include, every one a region contains for a region suppression, and the one
/// it names otherwise.
///
/// # Errors
///
/// When the change names an entity the part does not hold.
fn targets<M: Modality>(
    part: &PartId,
    entities: &[Entity<M>],
    change: &Change<M>,
) -> Result<Vec<usize>> {
    let position = |id| entities.iter().position(|entity| entity.id() == id);
    match change {
        Change::Include { .. } => Ok(Vec::new()),
        Change::SuppressWithin { region } => Ok((0..entities.len())
            .filter(|&i| covers(region, entities[i].location()))
            .collect()),
        Change::Suppress { entity: id }
        | Change::Relabel { entity: id, .. }
        | Change::Relocate { entity: id, .. } => position(*id)
            .map(|i| vec![i])
            .ok_or_else(|| Error::not_found(format!("part `{part}` holds no entity `{id}`"))),
    }
}

/// Whether `outer` covers all of `inner`: it contains it, or is the same
/// location.
fn covers<L: ModalityLocation>(outer: &L, inner: &L) -> bool {
    outer.overlap(inner) == Overlap::Contains || outer.hash() == inner.hash()
}
