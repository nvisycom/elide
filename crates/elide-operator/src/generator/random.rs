//! [`RandomToken`]: the default [`Generator`], a random token per entity.

use elide_core::entity::Entity;
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
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

impl Generator<Text> for RandomToken {
    fn generate(&self, _entity: &Entity<Text>, _data: &TextData) -> TextReplacement {
        TextReplacement::substituted(Uuid::new_v4().to_string())
    }
}

#[cfg(feature = "tabular")]
impl Generator<Tabular> for RandomToken {
    fn generate(&self, _entity: &Entity<Tabular>, _data: &TextData) -> TabularReplacement {
        TabularReplacement::Cell(TextReplacement::substituted(Uuid::new_v4().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::LabelRef;
    use elide_core::modality::text::{Text, TextLocation};

    use super::*;

    fn entity(label: &str) -> Entity<Text> {
        Entity::custom(LabelRef::new(label), TextLocation::new(0, 5)).build()
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
