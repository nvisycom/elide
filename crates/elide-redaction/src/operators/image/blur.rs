//! [`Blur`]: Gaussian-blur the matched image region.

use elide_core::entity::Entity;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Error, Result};
use elide_image::modality::{Image, ImageData, ImageReplacement};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Hides an image region under a Gaussian blur.
///
/// The soft visual treatment: the region is smeared but its rough shape
/// and surroundings stay legible, so the document still reads naturally.
/// Common for faces and license plates in low-stakes contexts. Contrast
/// [`Blackbox`], which hides the region behind a solid fill.
///
/// [`Blackbox`]: crate::operators::Blackbox
#[derive(Debug, Clone, Copy, Component)]
#[component(name = "blur", version = "1.0.0")]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "BlurConfig"))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Blur {
    /// Standard deviation of the Gaussian kernel, in pixels. Larger is
    /// blurrier (and harder to reverse).
    sigma: f32,
}

/// The smallest blur accepted, in pixels: a sub-pixel kernel leaves the region
/// readable.
const MIN_SIGMA: f32 = 1.0;

impl Blur {
    /// Blur with the given kernel standard deviation, in pixels.
    ///
    /// # Errors
    ///
    /// [`Configuration`](elide_core::ErrorKind::Configuration) if `sigma` is
    /// not finite or is below `1.0`: a sub-pixel kernel leaves the region
    /// readable.
    pub fn new(sigma: f32) -> Result<Self> {
        if !sigma.is_finite() || sigma < MIN_SIGMA {
            return Err(Error::configuration(format!(
                "blur sigma {sigma} must be a finite number of at least {MIN_SIGMA}"
            )));
        }
        Ok(Self { sigma })
    }
}

/// The deserialized form of a [`Blur`], validated through [`Blur::new`].
#[cfg(feature = "serde")]
#[derive(Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
struct BlurConfig {
    /// Standard deviation of the Gaussian kernel, in pixels.
    sigma: f32,
}

#[cfg(feature = "serde")]
impl TryFrom<BlurConfig> for Blur {
    type Error = Error;

    fn try_from(config: BlurConfig) -> Result<Self> {
        Self::new(config.sigma)
    }
}

impl Default for Blur {
    fn default() -> Self {
        // A moderate blur that obscures detail at typical document scales.
        Self { sigma: 16.0 }
    }
}

#[async_trait::async_trait]
impl Operator<Image> for Blur {
    fn leak_profile(&self) -> LeakProfile {
        // The region's position and bounding box stay observable, and a
        // light blur can leak coarse structure.
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Image>,
        _data: &ImageData,
    ) -> Result<Option<ImageReplacement>> {
        Ok(Some(ImageReplacement::Blur { sigma: self.sigma }))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use elide_core::entity::audit::{AuditEvent, PatternRule};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::primitive::Confidence;
    use elide_core::redaction::Operator;
    use elide_image::modality::{Image, ImageData, ImageLocation, ImageReplacement};
    use elide_image::primitive::{BoundingBox, Dimensions, Point};

    use crate::operators::{Blackbox, Blur, Pixelate};

    /// A 4x4 image entity with a small region, enough to drive an operator.
    fn image_entity() -> (Entity<Image>, ImageData) {
        let bbox = BoundingBox::from_origin(Point::new(0.0, 0.0), Dimensions::new(2.0, 2.0));
        let location = ImageLocation::new(bbox);
        let event = AuditEvent::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternRule::default(),
        );
        let entity = Entity::new(LabelRef::new("FACE"), location).with_event(event);
        let data = elide_image::fixtures::blank_image_data();
        (entity, data)
    }

    #[tokio::test]
    async fn image_operators_emit_their_replacement() {
        let (entity, data) = image_entity();

        let blur = Blur::new(8.0)
            .unwrap()
            .anonymize(&entity, &data)
            .await
            .unwrap();
        assert_eq!(blur, Some(ImageReplacement::Blur { sigma: 8.0 }));

        let pix = Pixelate::new(10)
            .unwrap()
            .anonymize(&entity, &data)
            .await
            .unwrap();
        assert_eq!(pix, Some(ImageReplacement::Pixelate { block_size: 10 }));

        let box_ = Blackbox::default().anonymize(&entity, &data).await.unwrap();
        assert_matches!(box_, Some(ImageReplacement::Block { .. }));
    }

    /// A blur or mosaic too fine to hide anything is rejected.
    #[test]
    fn settings_that_hide_nothing_are_rejected() {
        for sigma in [0.0, 0.5, -1.0, f32::NAN, f32::INFINITY] {
            assert!(Blur::new(sigma).is_err(), "sigma {sigma}");
        }
        for block_size in [0, 1] {
            assert!(Pixelate::new(block_size).is_err(), "block {block_size}");
        }
        assert!(Blur::new(1.0).is_ok());
        assert!(Pixelate::new(2).is_ok());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserializing_validates_the_settings() {
        assert!(serde_json::from_str::<Blur>(r#"{"sigma":0.0}"#).is_err());
        assert!(serde_json::from_str::<Pixelate>(r#"{"block_size":1}"#).is_err());
        let blur: Blur = serde_json::from_str(r#"{"sigma":4.0}"#).unwrap();
        assert_eq!(serde_json::to_string(&blur).unwrap(), r#"{"sigma":4.0}"#);
    }
}
