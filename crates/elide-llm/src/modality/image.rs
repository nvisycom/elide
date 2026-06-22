//! [`LlmModality`] for [`Image`]: scale each candidate's normalised box to
//! pixel space and build the entity.

use elide_core::entity::provenance::{Event, ModelEvent};
use elide_core::entity::{Entity, EntityCoRef, LabelRef};
use elide_core::modality::image::{Image, ImageData, ImageLocation};
use elide_core::primitive::{Confidence, UnitBoundingBox};

use super::{DEFAULT_CONFIDENCE, LlmModality};
use crate::candidates::{Candidates, ImageCandidate};

impl LlmModality for Image {
    type Item = ImageCandidate;

    fn lift(batch: Candidates<ImageCandidate>, data: &ImageData) -> Vec<Entity<Image>> {
        let dims = data.dimensions;

        let mut out = Vec::with_capacity(batch.entities.len());
        for d in batch.entities {
            let label = LabelRef::new(d.label.clone());
            let raw = d.confidence.unwrap_or(DEFAULT_CONFIDENCE);
            let Some(confidence) = Confidence::new(raw.clamp(0.0, 1.0) as f32) else {
                continue;
            };
            let bbox = UnitBoundingBox::from(d.bbox).denormalize(dims);
            let location = ImageLocation::new(bbox);
            let reason = format!("llm-image identified {}", label.as_str());
            let event = Event::model(
                "llm-image",
                confidence,
                location.clone(),
                ModelEvent {
                    name: "llm-image".into(),
                    ..ModelEvent::default()
                },
            )
            .with_reason(reason);
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
