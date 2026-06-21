//! Candidate to [`Entity`] lifting: localize each candidate and build the
//! final entity.
//!
//! Driven by the recognizer once a backend has extracted the candidate
//! batch. Text candidates carry a `value` + `context` that localize into a
//! byte range; image candidates carry a normalised bounding box scaled to
//! pixel space.

use elide_core::entity::provenance::{Event, ModelEvent};
use elide_core::entity::{Entity, EntityCoRef, LabelRef};
use elide_core::modality::image::{Image, ImageData, ImageLocation};
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::primitive::{Confidence, UnitBoundingBox};

use super::localize::{UnresolvedCandidatePolicy, localize_all};
use crate::candidates::{ImageCandidate, TextCandidate};

/// Default confidence assigned to a candidate when the model didn't score
/// it.
const DEFAULT_CONFIDENCE: f64 = 0.5;

/// Lift a text-candidate batch into `Entity<Text>` values: localize each
/// value into the source and build the entity.
pub(super) fn lift_text(data: &TextData, candidates: Vec<TextCandidate>) -> Vec<Entity<Text>> {
    let text = data.text.as_str();
    let localized = localize_all(text, candidates, UnresolvedCandidatePolicy::default());

    let mut out = Vec::with_capacity(localized.len());
    for l in localized {
        // The model's typed label is canonical; an untyped candidate is
        // dropped.
        let Some(model_label) = l.candidate.entity_type.as_deref() else {
            continue;
        };
        let label = LabelRef::new(model_label.to_owned());
        let raw = l.candidate.confidence.unwrap_or(DEFAULT_CONFIDENCE);
        let Some(confidence) = Confidence::new(raw.clamp(0.0, 1.0) as f32) else {
            continue;
        };
        let location = TextLocation::new(l.start_offset, l.end_offset);
        let reason = format!("llm-ner identified {}", label.as_str());
        let event = Event::model(
            "llm-ner",
            confidence,
            location.clone(),
            ModelEvent {
                name: "llm-ner".into(),
                ..ModelEvent::default()
            },
        )
        .with_reason(reason);

        let mut builder = Entity::builder()
            .with_label(label)
            .with_location(location)
            .with_confidence(confidence)
            .with_event(event);
        // The model groups mentions of the same real-world entity under a
        // shared id; carry it onto the entity as a coreference cluster.
        if let Some(id) = l.candidate.entity_id.clone() {
            builder = builder.with_coref(EntityCoRef::new(id));
        }
        out.push(builder.build().expect("required fields provided"));
    }
    out
}

/// Lift an image-candidate batch into `Entity<Image>` values: scale each
/// normalised box to pixel space and build the entity.
pub(super) fn lift_image(data: &ImageData, candidates: Vec<ImageCandidate>) -> Vec<Entity<Image>> {
    let dims = data.dimensions;

    let mut out = Vec::with_capacity(candidates.len());
    for d in candidates {
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
        let entity = Entity::builder()
            .with_label(label)
            .with_location(location)
            .with_confidence(confidence)
            .with_event(event)
            .build()
            .expect("required fields provided");
        out.push(entity);
    }
    out
}
