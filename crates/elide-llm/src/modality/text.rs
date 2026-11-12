//! [`LlmModality`] for [`Text`]: localize each text candidate into a byte
//! range and build the entity.

use elide_core::entity::audit::{AuditEvent, ModelId};
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::primitive::Confidence;

use super::localize::{UnresolvedCandidatePolicy, localize_all};
use super::{CorefScope, DEFAULT_CONFIDENCE, LlmModality};
use crate::candidates::{Candidates, TextCandidate};

impl LlmModality for Text {
    type Item = TextCandidate;

    fn lift(batch: Candidates<TextCandidate>, data: &TextData) -> Vec<Entity<Text>> {
        let text = data.text.as_str();
        let localized = localize_all(text, batch.entities, UnresolvedCandidatePolicy::default());

        let scope = CorefScope::new();
        let mut out = Vec::with_capacity(localized.len());
        for l in localized {
            let label = LabelRef::new(l.candidate.label.clone());
            let raw = l.candidate.confidence.unwrap_or(DEFAULT_CONFIDENCE);
            let Some(confidence) = Confidence::new(raw.clamp(0.0, 1.0) as f32) else {
                continue;
            };
            let location = TextLocation::inline(l.start_offset..l.end_offset);
            let event = AuditEvent::model(
                "llm-ner",
                confidence,
                location.clone(),
                ModelId {
                    name: "llm-ner".into(),
                    ..ModelId::default()
                },
            );

            let mut entity = Entity::new(label, location).with_event(event);
            // The model groups mentions of the same real-world entity under
            // a shared handle; scoped to this call, it is the entity's
            // coreference cluster.
            if let Some(handle) = &l.candidate.coreference {
                entity = entity.with_coref(scope.coref(handle));
            }
            out.push(entity);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(value: &str, coreference: &str) -> TextCandidate {
        TextCandidate {
            value: value.to_owned(),
            description: None,
            label: "person_name".to_owned(),
            confidence: Some(0.9),
            context: Some(value.to_owned()),
            coreference: Some(coreference.to_owned()),
        }
    }

    fn lift(text: &str, candidates: Vec<TextCandidate>) -> Vec<Entity<Text>> {
        Text::lift(
            Candidates {
                entities: candidates,
            },
            &TextData::new(text),
        )
    }

    /// One call's mentions of a cluster share an id; the same handle in
    /// another call is a different cluster.
    #[test]
    fn coreference_ids_are_scoped_to_the_call() {
        let text = "Alice met Bob";
        let first = lift(text, vec![candidate("Alice", "c1"), candidate("Bob", "c1")]);
        let second = lift(text, vec![candidate("Alice", "c1")]);

        assert_eq!(first[0].coref, first[1].coref, "one call, one cluster");
        assert_ne!(
            first[0].coref, second[0].coref,
            "calls never share a cluster"
        );
    }
}
