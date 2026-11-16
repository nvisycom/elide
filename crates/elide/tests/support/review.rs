//! A reviewer's flagged entity, for tests that apply a reviewed report.

use elide::entity::audit::{Manual, ManualIntent};
use elide::entity::{Entity, LabelRef};
use elide::modality::text::{Text, TextLocation};

/// An entity a reviewer flagged at `location` under `label`: one detection
/// missed, redacted like any detected one.
pub fn flagged(label: LabelRef, location: TextLocation) -> Entity<Text> {
    let mut entity = Entity::new(label, location.clone());
    entity.record_manual(Manual::new(ManualIntent::Flag, location), Some("reviewer"));
    entity
}
