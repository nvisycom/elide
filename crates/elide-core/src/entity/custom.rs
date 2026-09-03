//! [`CustomEntity`]: a reviewer-asserted entity, built with its attribution.

use super::{Entity, LabelRef};
use crate::entity::audit::{Attribution, AuditEvent, AuditLog, Manual, ManualIntent};
use crate::modality::Modality;
use crate::primitive::Confidence;

/// A user-asserted ("custom") entity under construction: one a reviewer marks
/// between detection and redaction, not produced by a recognizer.
///
/// Built by [`Entity::custom`]. Carries the two facts an add always has, a
/// `label` and a `location`, and optionally *who* asserted it ([`by`](Self::by))
/// and *why* ([`because`](Self::because)). On [`build`](Self::build) it stamps a
/// single [`Manual`](ManualIntent::Flag) audit event carrying both, so the human
/// origin, actor, and rationale are all on the one event, no double-stamping a
/// second event to attach attribution after the fact.
///
/// A `CustomEntity` is included directly: the engine's `Report::include_custom`
/// and its `_at` counterpart take `impl Into<Entity<M>>`, so the common path is a
/// single chain with no intermediate [`Entity`].
///
/// ```
/// # use elide_core::entity::{Entity, LabelRef};
/// # use elide_core::entity::audit::Attribution;
/// # use elide_core::modality::text::{Text, TextLocation};
/// let custom: Entity<Text> = Entity::custom(LabelRef::new("US_SSN"), TextLocation::new(0, 9))
///     .by("reviewer-7")
///     .because(Attribution::freeform("gdpr-art-17"))
///     .build();
/// ```
#[derive(Debug)]
pub struct CustomEntity<M: Modality> {
    label: LabelRef,
    location: M::Location,
    actor: Option<String>,
    attribution: Option<Attribution>,
}

impl<M: Modality> CustomEntity<M> {
    /// A custom add of `label` at `location`, unattributed. Prefer
    /// [`Entity::custom`], which calls this.
    pub(crate) fn new(label: impl Into<LabelRef>, location: M::Location) -> Self {
        Self {
            label: label.into(),
            location,
            actor: None,
            attribution: None,
        }
    }

    /// Record *who* asserted this entity: the reviewer's id, the audit event's
    /// source. Unset means an unattributed `"manual"` source.
    #[must_use]
    pub fn by(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }

    /// Record *why* this entity was asserted: the reviewer's [`Attribution`]
    /// (a policy rationale or citation), carried on the audit event.
    #[must_use]
    pub fn because(mut self, attribution: impl Into<Attribution>) -> Self {
        self.attribution = Some(attribution.into());
        self
    }

    /// Assemble the [`Entity`]: a fresh time-ordered id, [`MAX`](Confidence::MAX)
    /// confidence (a human assertion is certain), and one
    /// [`Manual`](ManualIntent::Flag) audit event carrying the actor and
    /// attribution, so its human origin is auditable and it is never mistaken for
    /// an automatic detection.
    #[must_use]
    pub fn build(self) -> Entity<M> {
        let mut manual = Manual::new(ManualIntent::Flag, self.location.clone());
        if let Some(attribution) = self.attribution {
            manual = manual.with_attribution(attribution);
        }
        let source = self.actor.unwrap_or_else(|| "manual".to_owned());
        let event = AuditEvent::manual(source, Confidence::MAX, manual);
        Entity::new(self.label, self.location, AuditLog::new(event))
    }
}

impl<M: Modality> From<CustomEntity<M>> for Entity<M> {
    fn from(custom: CustomEntity<M>) -> Self {
        custom.build()
    }
}
