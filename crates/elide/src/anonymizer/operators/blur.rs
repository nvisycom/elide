//! [`Blur`]: Gaussian-blur the matched image region.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::image::{Image, ImageData, ImageReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};

/// Gaussian-blur the matched image region.
///
/// The soft visual treatment: the region is smeared but its rough shape
/// and surroundings stay legible, so the document still reads naturally.
/// Common for faces and license plates in low-stakes contexts. Contrast
/// [`Blackbox`], which hides the region behind a solid fill.
///
/// [`Blackbox`]: super::Blackbox
#[derive(Debug, Clone, Copy)]
pub struct Blur {
    /// Standard deviation of the Gaussian kernel, in pixels. Larger is
    /// blurrier (and harder to reverse).
    sigma: f32,
}

impl Blur {
    /// Blur with the given kernel standard deviation, in pixels.
    pub fn new(sigma: f32) -> Self {
        Self { sigma }
    }
}

impl Default for Blur {
    fn default() -> Self {
        // A moderate blur that obscures detail at typical document scales.
        Self { sigma: 16.0 }
    }
}

impl Operator<Image> for Blur {
    fn id(&self) -> OperatorId {
        OperatorId::new("blur", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // The region's position and bounding box stay observable, and a
        // light blur can leak coarse structure.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &ImageData,
    ) -> Result<ImageReplacement> {
        Ok(ImageReplacement::Blur { sigma: self.sigma })
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::image::{Image, ImageData, ImageLocation, ImageReplacement};
    use elide_core::primitive::{BoundingBox, Confidence, Dimensions, Point};
    use elide_core::redaction::Operator;

    use super::super::{Blackbox, Pixelate};
    use super::Blur;

    /// A 4x4 image entity with a small region, enough to drive an operator.
    fn image_entity() -> (Entity<Image>, ImageData) {
        let bbox = BoundingBox::from_origin_size(Point::new(0.0, 0.0), 2.0, 2.0);
        let location = ImageLocation::new(bbox);
        let event = Event::pattern("t", Confidence::MAX, location.clone(), PatternEvent::default());
        let entity = Entity::new(
            LabelRef::new("FACE"),
            location,
            Confidence::MAX,
            Provenance::new(event),
        );
        let data = ImageData::new(vec![0u8; 4], Dimensions::new(4, 4));
        (entity, data)
    }

    #[tokio::test]
    async fn image_operators_emit_their_replacement() {
        let (entity, data) = image_entity();

        let blur = Blur::new(8.0).anonymize(&entity, &data).await.unwrap();
        assert_eq!(blur, ImageReplacement::Blur { sigma: 8.0 });

        let pix = Pixelate::new(10).anonymize(&entity, &data).await.unwrap();
        assert_eq!(pix, ImageReplacement::Pixelate { block_size: 10 });

        let box_ = Blackbox::default().anonymize(&entity, &data).await.unwrap();
        assert!(matches!(box_, ImageReplacement::Block { .. }));
    }
}
