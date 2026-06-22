//! [`Keep`]: pass the matched entity through unchanged.

use elide_core::Result;
use elide_core::entity::Entity;
#[cfg(feature = "audio")]
use elide_core::modality::audio::{Audio, AudioData, AudioReplacement};
#[cfg(feature = "image")]
use elide_core::modality::image::{Image, ImageData, ImageReplacement};
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::Tabular;
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Pass the matched entity through unchanged.
///
/// Useful in mixed policies: redact everything by default but keep, say,
/// currency amounts readable, or every face except the ones tagged to
/// retain. The replacement records "leave this as-is" so the entity still
/// shows up in the audit trail.
///
/// Works across modalities — a text/tabular span stays verbatim, an image
/// region or audio range is left untouched. Unlike the text operators,
/// `Keep` is implemented per modality rather than over [`TextBacked`], so
/// the one type serves every modality.
///
/// [`TextBacked`]: elide_core::modality::TextBacked
#[derive(Debug, Clone, Copy, Default)]
pub struct Keep;

/// The original value is unchanged: strictly the most leaky profile.
const KEEP_LEAK: LeakProfile = LeakProfile::Recoverable;

impl Operator<Text> for Keep {
    fn id(&self) -> OperatorId {
        OperatorId::new("keep", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        KEEP_LEAK
    }

    async fn anonymize(&self, _entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(data.as_str()))
    }
}

#[cfg(feature = "tabular")]
impl Operator<Tabular> for Keep {
    fn id(&self) -> OperatorId {
        OperatorId::new("keep", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        KEEP_LEAK
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(data.as_str()))
    }
}

#[cfg(feature = "image")]
impl Operator<Image> for Keep {
    fn id(&self) -> OperatorId {
        OperatorId::new("keep", "1.0.0")
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
impl Operator<Audio> for Keep {
    fn id(&self) -> OperatorId {
        OperatorId::new("keep", "1.0.0")
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
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
    use elide_core::primitive::Confidence;
    use elide_core::redaction::Operator;

    use super::Keep;

    fn text_entity() -> Entity<Text> {
        let location = TextLocation::new(0, 5);
        let event = Event::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternEvent::default(),
        );
        Entity::new(
            LabelRef::new("AMOUNT"),
            location,
            Confidence::MAX,
            Provenance::new(event),
        )
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
        use elide_core::modality::image::{Image, ImageData, ImageLocation, ImageReplacement};
        use elide_core::primitive::{BoundingBox, Dimensions, Point};

        let bbox = BoundingBox::from_origin_size(Point::new(0.0, 0.0), 2.0, 2.0);
        let location = ImageLocation::new(bbox);
        let event = Event::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternEvent::default(),
        );
        let entity: Entity<Image> = Entity::new(
            LabelRef::new("FACE"),
            location,
            Confidence::MAX,
            Provenance::new(event),
        );
        let data = ImageData::new(vec![0u8; 4], Dimensions::new(4, 4));

        let out = Keep.anonymize(&entity, &data).await.unwrap();
        assert_eq!(out, ImageReplacement::Unchanged);
    }
}
