//! [`LlmModality`] for [`Text`]: localize each text candidate into a byte
//! range and build the entity.

use elide_core::entity::provenance::{Event, ModelEvent};
use elide_core::entity::{Entity, EntityCoRef, LabelRef};
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::primitive::Confidence;

use super::localize::{UnresolvedCandidatePolicy, localize_all};
use super::{DEFAULT_CONFIDENCE, LlmModality};
use crate::candidates::{Candidates, TextCandidate};

impl LlmModality for Text {
    type Item = TextCandidate;

    fn lift(batch: Candidates<TextCandidate>, data: &TextData) -> Vec<Entity<Text>> {
        let text = data.text.as_str();
        let localized = localize_all(text, batch.entities, UnresolvedCandidatePolicy::default());

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
            // The model groups mentions of the same real-world entity under
            // a shared id; carry it onto the entity as a coreference cluster.
            if let Some(id) = l.candidate.entity_id.clone() {
                builder = builder.with_coref(EntityCoRef::new(id));
            }
            out.push(builder.build().expect("required fields provided"));
        }
        out
    }
}
