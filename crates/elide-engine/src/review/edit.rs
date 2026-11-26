//! [`Edit`] and [`Change`]: one reviewer decision about one part of a report.

use elide_core::entity::LabelRef;
use elide_core::entity::audit::Attribution;
use elide_core::modality::{Modality, ModalityLocation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::policy::Decision;

/// One reviewer decision about one part of a report: what it changes, and
/// why. Who made it is named when the review is applied.
///
/// Built through a [`ReviewPart`](super::ReviewPart), and applied by
/// [`Orchestrator::review`](crate::Orchestrator::review), which records it on
/// the entities it touches: an include as the event that starts its entity,
/// every other edit as a `Manual` decision.
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
    /// Why the edit was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub because: Option<Attribution>,
}

/// What an [`Edit`] changes.
///
/// An [`Include`](Self::Include) and a [`Confirm`](Self::Confirm) only add to
/// what redaction hides: they narrow nothing. The rest can leave content
/// unredacted that would have been hidden: a suppression, a relabel (another
/// label may get a weaker operator), and a relocation (a rule may choose its
/// operator by where an entity sits). The [`ReviewPolicy`](super::ReviewPolicy)
/// sees every edit, an include too, and can refuse any of them: one that wants
/// every edit attributed refuses an unattributed include or confirmation.
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
    /// Redact the entity `entity`, a finding the reviewer confirms: it is
    /// redacted even after an earlier suppression.
    Confirm {
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
    /// What the change does, for the [`ReviewPolicy`](super::ReviewPolicy).
    pub(crate) fn decision(&self) -> Decision<'_> {
        match self {
            Self::Include { label, .. } => Decision::Include { label },
            Self::Suppress { .. } => Decision::Suppress,
            Self::SuppressWithin { .. } => Decision::SuppressWithin,
            Self::Confirm { .. } => Decision::Confirm,
            Self::Relabel { label, .. } => Decision::Relabel { label },
            Self::Relocate { .. } => Decision::Relocate,
        }
    }

    /// Whether the change can leave content unredacted that would have been
    /// hidden: every change but an include and a confirmation.
    pub(crate) fn can_narrow(&self) -> bool {
        !matches!(self, Self::Include { .. } | Self::Confirm { .. })
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
            Self::Confirm { entity } => out.extend([vec![5], entity.as_bytes().to_vec()]),
        }
    }
}

impl<M: Modality> Edit<M> {
    /// The edit's fields, for the review's digest: its change, and why.
    pub(crate) fn fields(&self, out: &mut Vec<Vec<u8>>) {
        self.change.fields(out);
        out.push(match &self.because {
            Some(because) => [&[1], because.digest().as_bytes().as_slice()].concat(),
            None => vec![0],
        });
    }
}
