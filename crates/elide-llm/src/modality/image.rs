//! [`LlmModality`] for [`Image`]: scale each candidate's normalised box to
//! pixel space and build the entity.

use elide_core::entity::audit::{AuditEvent, ModelEvent};
use elide_core::entity::{Entity, EntityCoRef, LabelRef};
use elide_core::primitive::Confidence;
use elide_image::ImageBuffer;
use elide_image::modality::{Image, ImageData, ImageLocation};
use elide_image::primitive::UnitBoundingBox;

use super::{DEFAULT_CONFIDENCE, LlmModality};
use crate::candidates::{Candidates, ImageCandidate};

impl LlmModality for Image {
    type Item = ImageCandidate;

    fn lift(batch: Candidates<ImageCandidate>, data: &ImageData) -> Vec<Entity<Image>> {
        // The model reports boxes in normalised `0.0..=1.0` coordinates; scaling
        // them to pixels needs the image's pixel size, read from the container
        // header (the authoritative source, unlike a cached dimension that could
        // drift) — a header probe, not a full decode. If the bytes will not read
        // there is nothing to scale against, so no candidate can be placed.
        let Ok(dims) = ImageBuffer::dimensions_of(&data.bytes) else {
            return Vec::new();
        };

        let mut out = Vec::with_capacity(batch.entities.len());
        for d in batch.entities {
            let label = LabelRef::new(d.label.clone());
            let raw = d.confidence.unwrap_or(DEFAULT_CONFIDENCE);
            let Some(confidence) = Confidence::new(raw.clamp(0.0, 1.0) as f32) else {
                continue;
            };
            let bbox = UnitBoundingBox::from(d.bbox).denormalize(dims);
            let location = ImageLocation::new(bbox);
            let event = AuditEvent::model(
                "llm-image",
                confidence,
                location.clone(),
                ModelEvent {
                    name: "llm-image".into(),
                    ..ModelEvent::default()
                },
            );
            let mut builder = Entity::builder()
                .with_label(label)
                .with_location(location)
                .with_confidence(confidence)
                .with_event(event);
            // The model groups mentions of the same real-world entity under
            // a shared id; carry it onto the entity as a coreference cluster.
            if let Some(id) = d.coreference.clone() {
                builder = builder.with_coref(EntityCoRef::new(id));
            }
            out.push(builder.build().expect("required fields provided"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use elide_image::test_util;

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
        // 0..80, proving the dimensions come from decoding the bytes.
        let data = ImageData::new(test_util::png(100, 80));
        let batch = Candidates {
            entities: vec![candidate(0.0, 0.0, 0.5, 1.0)],
        };
        let entities = Image::lift(batch, &data);
        assert_eq!(entities.len(), 1);
        let bbox = entities[0].location.bounding_box;
        assert_eq!(bbox.min.x, 0.0);
        assert_eq!(bbox.max.x, 50.0);
        assert_eq!(bbox.max.y, 80.0);
    }

    #[test]
    fn lift_drops_everything_when_the_bytes_do_not_decode() {
        // With no pixel size to scale against, no candidate can be placed.
        let data = ImageData::new(b"not an image".to_vec());
        let batch = Candidates {
            entities: vec![candidate(0.0, 0.0, 0.5, 1.0)],
        };
        assert!(Image::lift(batch, &data).is_empty());
    }
}
