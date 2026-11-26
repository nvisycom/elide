//! [`AuditKind`]: the tagged union over the per-kind payload structs (in the
//! [`recognition`](super::recognition) / [`reconcile`](super::reconcile) /
//! [`apply`](super::apply) submodules), plus how a kind folds into the
//! tamper-evident hash.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::AuditHasher;
use super::{
    Annotated, Calibrated, Contested, Flagged, Fused, Included, Manual, Model, Pattern, Redacted,
    Refined, Resolved, Selected,
};
use crate::modality::Modality;

/// What kind of [`AuditEvent`] happened, with its event-specific detail.
///
/// It also carries the rationale for why it happened.
///
/// A thin tagged union: each variant wraps one payload struct (e.g.
/// [`Redacted`], [`Selected`], [`Manual`]) that owns that kind's fields, its
/// docs, and how it folds into the audit hash. Match on a variant to reach its
/// payload:
///
/// ```
/// # use elide_core::entity::audit::AuditKind;
/// # use elide_core::modality::text::Text;
/// # fn show(kind: &AuditKind<Text>) {
/// if let AuditKind::Redacted(redaction) = kind {
///     let _ = &redaction.selection;
/// }
/// # }
/// ```
///
/// `#[non_exhaustive]`: new event kinds can be added compatibly. The
/// recognition kinds ([`Pattern`], [`Model`]) carry the matched [`Location`];
/// the rest carry their own data.
///
/// [`AuditEvent`]: super::AuditEvent
/// [`Pattern`]: AuditKind::Pattern
/// [`Model`]: AuditKind::Model
/// [`Location`]: Modality::Location
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(
        tag = "kind",
        content = "detail",
        rename_all = "snake_case",
        bound = "M::Location: Serialize + for<'a> Deserialize<'a>"
    )
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema",
        rename = "{M}AuditKind"
    )
)]
#[non_exhaustive]
pub enum AuditKind<M: Modality> {
    /// A pattern or dictionary recognizer matched here.
    Pattern(Pattern<M>),
    /// A model / NER recognizer matched here.
    Model(Model<M>),
    /// An item of hidden content its format labels (a metadata field, a
    /// comment, an author, a tracked deletion) was flagged here as a redaction
    /// subject.
    Flagged(Flagged<M>),
    /// Several detections were fused into one entity.
    Fused(Fused<M>),
    /// A competing detection of a different label over the same span was
    /// resolved against this (winning) entity.
    Resolved(Resolved),
    /// A competing detection of a different label over the same span was left
    /// *unresolved*: both entities survive, flagged for a human to settle.
    Contested(Contested),
    /// The entity's confidence was rescaled by a per-recognizer factor.
    Calibrated(Calibrated),
    /// A context keyword near the entity lifted its confidence.
    Refined(Refined<M>),
    /// An operator hid the entity.
    Redacted(Redacted<M>),
    /// An operator was *picked* to hide the entity, the redaction decision,
    /// recorded before it is applied so it can be reviewed first.
    Selected(Selected),
    /// A reviewer included an entity detection missed.
    Included(Included<M>),
    /// A reviewer's decision about an entity: a verdict on whether it is
    /// redacted, or an amendment of its label or location.
    Manual(Manual<M>),
    /// Something became known about the entity besides a decision: where it
    /// sits in another coordinate space, where it matched in the text it was
    /// recognized in, its language, or its coreference cluster.
    Annotated(Annotated<M>),
}

impl<M: Modality> AuditKind<M> {
    /// This event kind's identifying bytes, for the audit hash.
    ///
    /// A leading discriminant byte tags the variant, then each field is folded
    /// in (locations via [`ModalityLocation::hash`]). No serialization and no
    /// `serde` bound: the tamper-evident chain hashes the same in every build.
    ///
    /// [`ModalityLocation::hash`]: crate::modality::ModalityLocation::hash
    pub(crate) fn hash_into(&self, out: &mut AuditHasher) {
        // Each arm writes its payload's central discriminant `TAG`, then folds
        // the payload's own fields in. The per-kind logic (the `TAG` and the
        // `hash_into`) lives with the kind's struct in the `recognition`,
        // `reconcile`, and `apply` submodules; here it is uniform "tag, then
        // body" dispatch.
        match self {
            Self::Pattern(p) => {
                out.byte(Pattern::<M>::TAG);
                p.hash_into(out);
            }
            Self::Flagged(f) => {
                out.byte(Flagged::<M>::TAG);
                f.hash_into(out);
            }
            Self::Model(m) => {
                out.byte(Model::<M>::TAG);
                m.hash_into(out);
            }
            Self::Fused(d) => {
                out.byte(Fused::<M>::TAG);
                d.hash_into(out);
            }
            Self::Resolved(c) => {
                out.byte(Resolved::TAG);
                c.hash_into(out);
            }
            Self::Contested(c) => {
                out.byte(Contested::TAG);
                c.hash_into(out);
            }
            Self::Calibrated(c) => {
                out.byte(Calibrated::TAG);
                c.hash_into(out);
            }
            Self::Refined(r) => {
                out.byte(Refined::<M>::TAG);
                r.hash_into(out);
            }
            Self::Redacted(r) => {
                out.byte(Redacted::<M>::TAG);
                r.hash_into(out);
            }
            Self::Selected(s) => {
                out.byte(Selected::TAG);
                s.hash_into(out);
            }
            Self::Included(i) => {
                out.byte(Included::<M>::TAG);
                i.hash_into(out);
            }
            Self::Manual(m) => {
                out.byte(Manual::<M>::TAG);
                m.hash_into(out);
            }
            Self::Annotated(a) => {
                out.byte(Annotated::<M>::TAG);
                a.hash_into(out);
            }
        }
    }
}

// Each payload converts into its `AuditKind` variant, so a built payload flows
// into `AuditEvent::new(source, confidence, payload.into())` without naming the
// variant.

impl<M: Modality> From<Pattern<M>> for AuditKind<M> {
    fn from(payload: Pattern<M>) -> Self {
        Self::Pattern(payload)
    }
}

impl<M: Modality> From<Model<M>> for AuditKind<M> {
    fn from(payload: Model<M>) -> Self {
        Self::Model(payload)
    }
}

impl<M: Modality> From<Flagged<M>> for AuditKind<M> {
    fn from(payload: Flagged<M>) -> Self {
        Self::Flagged(payload)
    }
}

impl<M: Modality> From<Fused<M>> for AuditKind<M> {
    fn from(payload: Fused<M>) -> Self {
        Self::Fused(payload)
    }
}

impl<M: Modality> From<Resolved> for AuditKind<M> {
    fn from(payload: Resolved) -> Self {
        Self::Resolved(payload)
    }
}

impl<M: Modality> From<Contested> for AuditKind<M> {
    fn from(payload: Contested) -> Self {
        Self::Contested(payload)
    }
}

impl<M: Modality> From<Calibrated> for AuditKind<M> {
    fn from(payload: Calibrated) -> Self {
        Self::Calibrated(payload)
    }
}

impl<M: Modality> From<Refined<M>> for AuditKind<M> {
    fn from(payload: Refined<M>) -> Self {
        Self::Refined(payload)
    }
}

impl<M: Modality> From<Redacted<M>> for AuditKind<M> {
    fn from(payload: Redacted<M>) -> Self {
        Self::Redacted(payload)
    }
}

impl<M: Modality> From<Selected> for AuditKind<M> {
    fn from(payload: Selected) -> Self {
        Self::Selected(payload)
    }
}

impl<M: Modality> From<Included<M>> for AuditKind<M> {
    fn from(payload: Included<M>) -> Self {
        Self::Included(payload)
    }
}

impl<M: Modality> From<Manual<M>> for AuditKind<M> {
    fn from(payload: Manual<M>) -> Self {
        Self::Manual(payload)
    }
}

impl<M: Modality> From<Annotated<M>> for AuditKind<M> {
    fn from(payload: Annotated<M>) -> Self {
        Self::Annotated(payload)
    }
}
