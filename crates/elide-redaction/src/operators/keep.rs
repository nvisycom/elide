//! [`Keep`]: pass the matched entity through unchanged.

#[cfg(feature = "audio")]
use elide_audio::modality::{Audio, AudioData, AudioReplacement};
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::primitive::ComponentId;
use elide_core::redaction::{LeakProfile, Operator};
#[cfg(feature = "image")]
use elide_image::modality::{Image, ImageData, ImageReplacement};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::operators::TextValued;

/// Passes an entity through unchanged, to exempt it from redaction.
///
/// Useful in mixed policies: redact everything by default but keep, say,
/// currency amounts readable, or every face except the ones tagged to
/// retain. The replacement records "leave this as-is" so the entity still
/// shows up in the audit trail.
///
/// Works across modalities: a text or tabular span stays verbatim, an image
/// region or audio range is left untouched, and a metadata field keeps its
/// value.
///
/// A field that stands for a group with others (an EXIF device or GPS field)
/// goes with the group when another of its fields is redacted: removal wins,
/// since what the group's other tags reveal is why the group is cleared.
/// The field's audit still shows the selection of `Keep`.
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Keep;

/// The original value is unchanged: strictly the most leaky profile.
const KEEP_LEAK: LeakProfile = LeakProfile::Recoverable;

impl Keep {
    /// Identity shared by every modality's impl.
    fn id() -> ComponentId {
        ComponentId::new("keep", "1.0.0")
    }
}

#[async_trait::async_trait]
impl<M: TextValued> Operator<M> for Keep {
    fn id(&self) -> ComponentId {
        Keep::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        KEEP_LEAK
    }

    async fn anonymize(&self, _entity: &Entity<M>, data: &M::Data) -> Result<M::Replacement> {
        Ok(M::unchanged(data))
    }
}

#[cfg(feature = "image")]
#[async_trait::async_trait]
impl Operator<Image> for Keep {
    fn id(&self) -> ComponentId {
        Keep::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        KEEP_LEAK
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &ImageData,
    ) -> Result<ImageReplacement> {
        Ok(ImageReplacement::Unchanged)
    }
}

#[cfg(feature = "audio")]
#[async_trait::async_trait]
impl Operator<Audio> for Keep {
    fn id(&self) -> ComponentId {
        Keep::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        KEEP_LEAK
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Audio>,
        _data: &AudioData,
    ) -> Result<AudioReplacement> {
        Ok(AudioReplacement::Unchanged)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, PatternRule};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
    use elide_core::primitive::Confidence;
    use elide_core::redaction::Operator;

    use super::Keep;

    fn text_entity() -> Entity<Text> {
        let location = TextLocation::inline(0..5);
        let event = AuditEvent::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternRule::default(),
        );
        Entity::new(LabelRef::new("AMOUNT"), location).with_event(event)
    }

    #[tokio::test]
    async fn text_keep_returns_the_original_verbatim() {
        let out = Keep
            .anonymize(&text_entity(), &TextData::new("$1,000"))
            .await
            .unwrap();
        assert_eq!(out, TextReplacement::substituted("$1,000"));
    }

    #[cfg(feature = "image")]
    #[tokio::test]
    async fn image_keep_leaves_the_region_unchanged() {
        use elide_image::modality::{Image, ImageLocation, ImageReplacement};
        use elide_image::primitive::{BoundingBox, Dimensions, Point};

        let bbox = BoundingBox::from_origin(Point::new(0.0, 0.0), Dimensions::new(2.0, 2.0));
        let location = ImageLocation::new(bbox);
        let event = AuditEvent::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternRule::default(),
        );
        let entity: Entity<Image> = Entity::new(LabelRef::new("FACE"), location).with_event(event);
        let data = elide_image::fixtures::blank_image_data();

        let out = Keep.anonymize(&entity, &data).await.unwrap();
        assert_eq!(out, ImageReplacement::Unchanged);
    }
}
