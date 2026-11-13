//! [`LlmModality`] for [`Text`]: localize each text candidate into a byte
//! range and build the entity.

use elide_core::entity::Entity;
use elide_core::entity::audit::ModelId;
use elide_core::modality::text::{Text, TextData, TextLocation};

use super::localize::localize_all;
use super::{Lifter, LlmModality};
use crate::candidates::{Candidates, TextCandidate};

impl LlmModality for Text {
    type Item = TextCandidate;

    fn lift(
        batch: Candidates<TextCandidate>,
        data: &TextData,
        model: &ModelId,
    ) -> Vec<Entity<Text>> {
        let lifter = Lifter::new("llm-ner", model);
        localize_all(data.text.as_str(), batch.entities)
            .into_iter()
            .filter_map(|localized| {
                let candidate = localized.candidate;
                lifter.entity(
                    candidate.label,
                    candidate.confidence,
                    TextLocation::inline(localized.range),
                    candidate.coreference.as_deref(),
                )
            })
            .collect()
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
            &ModelId::default(),
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
