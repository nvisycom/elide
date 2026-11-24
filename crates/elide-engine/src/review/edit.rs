//! [`Edit`] and [`Change`]: one reviewer decision about one part of a report.

use elide_core::entity::LabelRef;
use elide_core::entity::audit::{Attribution, ManualIntent};
use elide_core::modality::{Modality, ModalityLocation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One reviewer decision about one part of a report: what it changes, who
/// made it, and why.
///
/// Built through a [`ReviewPart`](super::ReviewPart), and applied by
/// [`Orchestrator::review`](crate::Orchestrator::review), which records it on
/// the entities it touches as a `Manual` audit event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound = "M::Location: Serialize + for<'a> Deserialize<'a>")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Edit"
    )
)]
pub struct Edit<M: Modality> {
    /// What the edit changes.
    #[serde(flatten)]
    pub change: Change<M>,
    /// Who made the edit, the audit event's source; `None` records it as
    /// `"manual"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Why the edit was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub because: Option<Attribution>,
}

/// What an [`Edit`] changes.
///
/// An [`Include`](Self::Include) only adds to what redaction hides: it narrows
/// nothing. The rest can leave content unredacted that would have been hidden:
/// a suppression, a relabel (another label may get a weaker operator), and a
/// relocation (a rule may choose its operator by where an entity sits). The
/// [`ReviewPolicy`](super::ReviewPolicy) sees every edit, an include too, and
/// can refuse any of them: one that wants every edit attributed refuses an
/// unattributed include.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "snake_case",
    bound = "M::Location: Serialize + for<'a> Deserialize<'a>"
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        tag = "op",
        rename_all = "snake_case",
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}Change"
    )
)]
pub enum Change<M: Modality> {
    /// Add an entity of `label` at `location`, one detection missed, with
    /// the id `entity`, so later edits can address it.
    Include {
        /// The new entity's id: one no entity of the report holds, in any
        /// part, and no other include of the review gives.
        entity: Uuid,
        /// The label of the entity.
        label: LabelRef,
        /// Where it sits.
        location: M::Location,
    },
    /// Leave the entity `entity` unredacted.
    Suppress {
        /// The entity's id.
        entity: Uuid,
    },
    /// Leave every entity `region` contains unredacted. An entity reaching
    /// outside it is kept, so nothing outside the region is spared.
    SuppressWithin {
        /// The region whose entities are suppressed.
        region: M::Location,
    },
    /// Give the entity `entity` the label `label`.
    Relabel {
        /// The entity's id.
        entity: Uuid,
        /// Its new label.
        label: LabelRef,
    },
    /// Move the entity `entity` to `location`.
    Relocate {
        /// The entity's id.
        entity: Uuid,
        /// Its new location.
        location: M::Location,
    },
}

impl<M: Modality> Change<M> {
    /// The reviewer decision the change records on the entities it touches.
    pub(crate) fn intent(&self) -> ManualIntent {
        match self {
            Self::Include { label, .. } => ManualIntent::Include {
                label: label.clone(),
            },
            Self::Suppress { .. } | Self::SuppressWithin { .. } => ManualIntent::Suppress,
            Self::Relabel { label, .. } => ManualIntent::Relabel {
                label: label.clone(),
            },
            Self::Relocate { .. } => ManualIntent::Relocate,
        }
    }

    /// The change's fields, for the review's digest: a tag, then each field.
    fn fields(&self, out: &mut Vec<Vec<u8>>) {
        match self {
            Self::Include {
                entity,
                label,
                location,
            } => {
                out.extend([
                    vec![0],
                    entity.as_bytes().to_vec(),
                    label.as_str().as_bytes().to_vec(),
                    location.hash(),
                ]);
            }
            Self::Suppress { entity } => out.extend([vec![1], entity.as_bytes().to_vec()]),
            Self::SuppressWithin { region } => out.extend([vec![2], region.hash()]),
            Self::Relabel { entity, label } => out.extend([
                vec![3],
                entity.as_bytes().to_vec(),
                label.as_str().as_bytes().to_vec(),
            ]),
            Self::Relocate { entity, location } => {
                out.extend([vec![4], entity.as_bytes().to_vec(), location.hash()]);
            }
        }
    }
}

impl<M: Modality> Edit<M> {
    /// The edit's fields, for the review's digest: its change, who made it,
    /// and why.
    pub(crate) fn fields(&self, out: &mut Vec<Vec<u8>>) {
        self.change.fields(out);
        out.push(match &self.by {
            Some(by) => [&[1], by.as_bytes()].concat(),
            None => vec![0],
        });
        out.push(match &self.because {
            Some(because) => [&[1], because.digest().as_bytes().as_slice()].concat(),
            None => vec![0],
        });
    }
}
