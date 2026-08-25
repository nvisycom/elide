//! [`AuditKind`]: the tagged union over the per-kind payload structs (in the
//! [`recognition`](super::recognition) / [`reconcile`](super::reconcile) /
//! [`apply`](super::apply) submodules), plus how a kind folds into the
//! tamper-evident hash.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::super::hash::AuditHasher;
use super::{
    Calibration, Conflict, Contested, Deduplication, Manual, Model, Pattern, Redaction, Refinement,
    Selection,
};
use crate::modality::Modality;

/// Kind of an [`AuditEvent`], carrying its event-specific detail and the
/// rationale for why it happened.
///
/// A thin tagged union: each variant wraps one payload struct (e.g.
/// [`Redaction`], [`Selection`], [`Manual`]) that owns that kind's fields, its
/// docs, and how it folds into the audit hash. Match on a variant to reach its
/// payload:
///
/// ```
/// # use elide_core::entity::audit::AuditKind;
/// # use elide_core::modality::text::Text;
/// # fn show(kind: &AuditKind<Text>) {
/// if let AuditKind::Redaction(redaction) = kind {
///     let _ = &redaction.operator;
/// }
/// # }
/// ```
///
/// `#[non_exhaustive]`: new event kinds (verification, annotation, …) can be
/// added compatibly. The recognition kinds ([`Pattern`], [`Model`]) carry the
/// matched [`Location`]; the rest carry their own data.
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
        bound = "M::Location: Serialize + for<'a> Deserialize<'a>, \
                 M::Data: Serialize + for<'a> Deserialize<'a>"
    )
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        bound = "M: schemars::JsonSchema, M::Location: schemars::JsonSchema, M::Data: schemars::JsonSchema",
        rename = "{M}AuditKind"
    )
)]
#[non_exhaustive]
pub enum AuditKind<M: Modality> {
    /// A pattern or dictionary recognizer matched here.
    Pattern(Pattern<M>),
    /// A model / NER recognizer matched here.
    Model(Model<M>),
    /// Several detections were fused into one entity.
    Deduplication(Deduplication),
    /// A competing detection of a different label over the same span was
    /// resolved against this (winning) entity.
    Conflict(Conflict),
    /// A competing detection of a different label over the same span was left
    /// *unresolved*: both entities survive, flagged for a human to settle.
    Contested(Contested),
    /// The entity's confidence was rescaled by a per-recognizer factor.
    Calibration(Calibration),
    /// A context keyword near the entity lifted its confidence.
    Refinement(Refinement<M>),
    /// An operator hid the entity.
    Redaction(Redaction),
    /// An operator was *picked* to hide the entity — the redaction decision,
    /// recorded before it is applied so it can be reviewed first.
    Selection(Selection),
    /// A human override, outside automatic detection: an entity a reviewer
    /// added by hand, or a detected one they marked to ignore.
    Manual(Manual<M>),
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
        // `hash_into`) lives with the kind's struct in [`payload`](super::payload);
        // here it is uniform "tag, then body" dispatch.
        match self {
            Self::Pattern(p) => {
                out.byte(Pattern::<M>::TAG);
                p.hash_into(out);
            }
            Self::Model(m) => {
                out.byte(Model::<M>::TAG);
                m.hash_into(out);
            }
            Self::Deduplication(d) => {
                out.byte(Deduplication::TAG);
                d.hash_into(out);
            }
            Self::Conflict(c) => {
                out.byte(Conflict::TAG);
                c.hash_into(out);
            }
            Self::Contested(c) => {
                out.byte(Contested::TAG);
                c.hash_into(out);
            }
            Self::Calibration(c) => {
                out.byte(Calibration::TAG);
                c.hash_into(out);
            }
            Self::Refinement(r) => {
                out.byte(Refinement::<M>::TAG);
                r.hash_into(out);
            }
            Self::Redaction(r) => {
                out.byte(Redaction::TAG);
                r.hash_into(out);
            }
            Self::Selection(s) => {
                out.byte(Selection::TAG);
                s.hash_into(out);
            }
            Self::Manual(m) => {
                out.byte(Manual::<M>::TAG);
                m.hash_into(out);
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

impl<M: Modality> From<Deduplication> for AuditKind<M> {
    fn from(payload: Deduplication) -> Self {
        Self::Deduplication(payload)
    }
}

impl<M: Modality> From<Conflict> for AuditKind<M> {
    fn from(payload: Conflict) -> Self {
        Self::Conflict(payload)
    }
}

impl<M: Modality> From<Contested> for AuditKind<M> {
    fn from(payload: Contested) -> Self {
        Self::Contested(payload)
    }
}

impl<M: Modality> From<Calibration> for AuditKind<M> {
    fn from(payload: Calibration) -> Self {
        Self::Calibration(payload)
    }
}

impl<M: Modality> From<Refinement<M>> for AuditKind<M> {
    fn from(payload: Refinement<M>) -> Self {
        Self::Refinement(payload)
    }
}

impl<M: Modality> From<Redaction> for AuditKind<M> {
    fn from(payload: Redaction) -> Self {
        Self::Redaction(payload)
    }
}

impl<M: Modality> From<Selection> for AuditKind<M> {
    fn from(payload: Selection) -> Self {
        Self::Selection(payload)
    }
}

impl<M: Modality> From<Manual<M>> for AuditKind<M> {
    fn from(payload: Manual<M>) -> Self {
        Self::Manual(payload)
    }
}
