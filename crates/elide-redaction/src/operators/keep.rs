//! [`Keep`]: pass the matched entity through unchanged.

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Passes an entity through unchanged, to exempt it from redaction.
///
/// Useful in mixed policies: redact everything by default but keep, say,
/// currency amounts readable, or every face except the ones tagged to
/// retain. It writes nothing, in any modality, so the document is left as it
/// is there. The entity's audit trail records that `Keep` was selected, and no
/// redaction, since nothing was hidden.
///
/// A field that stands for a group with others (an EXIF device or GPS field)
/// goes with the group when another of its fields is redacted: removal wins,
/// since what the group's other tags reveal is why the group is cleared.
/// The field's audit still shows the selection of `Keep`.
#[derive(Debug, Clone, Copy, Default, Component)]
#[component(name = "keep", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Keep;

#[async_trait::async_trait]
impl<M: Modality> Operator<M> for Keep {
    /// The original value is unchanged: strictly the most leaky profile.
    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Recoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<M>,
        _data: &M::Data,
    ) -> Result<Option<M::Replacement>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, PatternRule};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextData, TextLocation};
    use elide_core::primitive::Confidence;
    use elide_core::redaction::Operator;

    use super::Keep;

    #[tokio::test]
    async fn keep_writes_nothing() {
        let location = TextLocation::inline(0..6);
        let event = AuditEvent::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternRule::default(),
        );
        let entity: Entity<Text> = Entity::new(LabelRef::new("AMOUNT"), location).with_event(event);
        let out = Keep
            .anonymize(&entity, &TextData::new("$1,000"))
            .await
            .unwrap();
        assert_eq!(out, None);
    }
}
