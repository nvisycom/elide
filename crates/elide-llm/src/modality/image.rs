//! [`LlmModality`] for [`Image`]: scale each candidate's normalised box to
//! pixel space and build the entity.

use elide_core::entity::Entity;
use elide_core::entity::audit::ModelId;
use elide_image::modality::{Image, ImageData, ImageLocation};
use elide_image::primitive::UnitBoundingBoxExt;

use super::{Lifter, LlmModality};
use crate::candidates::{Candidates, ImageCandidate};

const TARGET: &str = "elide_llm::modality::image";

impl LlmModality for Image {
    type Item = ImageCandidate;

    fn lift(
        batch: Candidates<ImageCandidate>,
        data: &ImageData,
        model: &ModelId,
    ) -> Vec<Entity<Image>> {
        // The model reports boxes in normalised `0.0..=1.0` coordinates; scaling
        // them to pixels needs the image's pixel size, an intrinsic fact of the
        // decoded payload.
        let dims = data.dimensions();
        let lifter = Lifter::new("llm-image", model);
        batch
            .entities
            .into_iter()
            .filter_map(|candidate| {
                let Some(bbox) = candidate.bbox.to_unit_bounding_box() else {
                    tracing::warn!(
                        target: TARGET,
                        label = %candidate.label,
                        bbox = ?candidate.bbox,
                        "dropping image candidate with an empty or non-finite box",
                    );
                    return None;
                };
                lifter.entity(
                    candidate.label,
                    candidate.confidence,
                    ImageLocation::new(bbox.denormalize(dims)),
                    candidate.coreference.as_deref(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use elide_image::fixtures;

    use super::*;
    use crate::candidates::UnitBox;

    fn candidate(x_min: f64, y_min: f64, x_max: f64, y_max: f64) -> ImageCandidate {
        ImageCandidate {
            bbox: UnitBox {
                x_min,
                y_min,
                x_max,
                y_max,
            },
            label: "person".to_owned(),
            confidence: Some(0.9),
            description: None,
            coreference: None,
        }
    }

    #[test]
    fn lift_denormalises_boxes_using_the_decoded_dimensions() {
        // A box over the left half of a 100x80 image scales to pixels 0..50 x
        // 0..80, from the payload's intrinsic dimensions.
        let data = fixtures::image_data(
            fixtures::png(100, 80),
            elide_image::modality::ImageFormat::Png,
        );
        let batch = Candidates {
            entities: vec![candidate(0.0, 0.0, 0.5, 1.0)],
        };
        let entities = Image::lift(batch, &data, &ModelId::default());
        assert_eq!(entities.len(), 1);
        let bbox = entities[0].location.bounding_box;
        assert_eq!(bbox.min.x, 0.0);
        assert_eq!(bbox.max.x, 50.0);
        assert_eq!(bbox.max.y, 80.0);
    }

    /// Swapped corners and coordinates outside the image still cover the
    /// region the model meant, clamped to the image.
    #[test]
    fn lift_orders_corners_and_clamps_to_the_image() {
        let data = fixtures::image_data(
            fixtures::png(100, 80),
            elide_image::modality::ImageFormat::Png,
        );
        let batch = Candidates {
            entities: vec![candidate(0.5, 1.2, -0.1, 0.0)],
        };
        let bbox = Image::lift(batch, &data, &ModelId::default())[0]
            .location
            .bounding_box;
        assert_eq!((bbox.min.x, bbox.min.y), (0.0, 0.0));
        assert_eq!((bbox.max.x, bbox.max.y), (50.0, 80.0));
    }

    /// A box with no area, or a non-finite coordinate, redacts nothing: it is
    /// dropped.
    #[test]
    fn lift_drops_empty_and_non_finite_boxes() {
        let data = fixtures::image_data(
            fixtures::png(100, 80),
            elide_image::modality::ImageFormat::Png,
        );
        let batch = Candidates {
            entities: vec![
                candidate(0.3, 0.3, 0.3, 0.9),
                candidate(f64::NAN, 0.0, 0.5, 0.5),
            ],
        };
        assert!(Image::lift(batch, &data, &ModelId::default()).is_empty());
    }
}
