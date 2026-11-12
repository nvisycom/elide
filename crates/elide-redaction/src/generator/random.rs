//! [`RandomToken`]: the default [`Generator`], a random token per entity.

use elide_core::entity::Entity;
use elide_core::modality::TextRedactable;
use elide_core::modality::text::TextReplacement;
use elide_core::redaction::Generator;
use uuid::Uuid;

/// Default text [`Generator`]: a fresh random token per distinct entity.
///
/// Emits a random UUID (`v4`) as the surrogate. It carries no resemblance
/// to a real value, the point is only that distinct entities get
/// distinct, unguessable stand-ins; consistency across mentions is the
/// vault's job, not the token's. Swap in a generator that produces
/// believable names or addresses (e.g. `elide-fake`'s `FakeGenerator`) when
/// readability matters more than opacity.
#[derive(Debug, Clone, Copy, Default)]
pub struct RandomToken;

impl<M: TextRedactable> Generator<M> for RandomToken {
    fn generate(&self, _entity: &Entity<M>, _data: &M::Data) -> M::Replacement {
        M::Replacement::from(TextReplacement::substituted(Uuid::new_v4().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::LabelRef;
    use elide_core::modality::text::{Text, TextData, TextLocation};

    use super::*;

    fn entity(label: &str) -> Entity<Text> {
        Entity::new(LabelRef::new(label), TextLocation::inline(0..5))
    }

    fn surrogate(label: &str) -> TextReplacement {
        let entity = entity(label);
        RandomToken.generate(&entity, &TextData::new("alice"))
    }

    #[test]
    fn produces_distinct_random_tokens() {
        // Each call mints a fresh token; the vault, not the generator,
        // makes repeats consistent.
        assert_ne!(surrogate("PERSON"), surrogate("PERSON"));
    }

    #[test]
    fn token_is_a_substituted_value() {
        let TextReplacement::Substituted(token) = surrogate("PERSON") else {
            panic!("expected a substituted token");
        };
        assert!(!token.is_empty());
    }
}
