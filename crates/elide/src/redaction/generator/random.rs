//! [`RandomToken`]: the default [`Generator`], a random token per entity.

use elide_core::entity::LabelRef;
use elide_core::modality::TextBacked;
use elide_core::modality::text::TextReplacement;
use uuid::Uuid;

use super::Generator;

/// Default text [`Generator`]: a fresh random token per distinct entity.
///
/// Emits a random UUID (`v4`) as the surrogate. It carries no resemblance
/// to a real value — the point is only that distinct entities get
/// distinct, unguessable stand-ins; consistency across mentions is the
/// vault's job, not the token's. Swap in a generator that produces
/// believable names or addresses when readability matters more than
/// opacity.
#[derive(Debug, Clone, Copy, Default)]
pub struct RandomToken;

impl<M: TextBacked> Generator<M> for RandomToken {
    fn generate(&self, _label: &LabelRef, _seed: &str) -> TextReplacement {
        TextReplacement::substituted(Uuid::new_v4().to_string())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::modality::text::Text;

    use super::*;

    fn surrogate(label: &str, seed: &str) -> TextReplacement {
        Generator::<Text>::generate(&RandomToken, &LabelRef::new(label), seed)
    }

    #[test]
    fn produces_distinct_random_tokens() {
        // Each call mints a fresh token; the vault, not the generator,
        // makes repeats consistent.
        assert_ne!(surrogate("PERSON", "alice"), surrogate("PERSON", "alice"));
    }

    #[test]
    fn token_is_a_substituted_value() {
        let TextReplacement::Substituted(token) = surrogate("PERSON", "alice") else {
            panic!("expected a substituted token");
        };
        assert!(!token.is_empty());
    }
}
