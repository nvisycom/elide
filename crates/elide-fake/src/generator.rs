//! [`FakeGenerator`]: a [`Generator`] that mints believable, label-aware values.

use std::str::FromStr;

use elide_core::entity::Entity;
use elide_core::modality::Modality;
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::primitive::LanguageTag;
use elide_core::redaction::Generator;

use crate::synth::{fake_value, fallback_token};

/// A [`Generator`] that produces believable, locale-aware, label-aware
/// surrogates, the readable alternative to an opaque UUID token.
///
/// Where `RandomToken` mints a UUID with no resemblance to the original, this
/// dispatches on the entity's label: a `person_name` becomes a plausible name,
/// an `email_address` a fresh address, a `phone_number` a same-shape number, and
/// so on. Structured labels (email, phone, postal code, dates, cards, IBAN)
/// pattern-preserve the original read from the entity's data, matching its
/// length and character-class layout with randomised content; free-form labels
/// (names, organisations, addresses) emit a fresh locale-aware value.
///
/// Drop it into the pseudonymizing operator for readable, still-recoverable
/// output: `Pseudonymize::new(vault, FakeGenerator::new())`.
///
/// Coreference-consistent by construction: the surrogate is seeded from the
/// entity's coreference id (or its UUID when uncorefed), so coreferent mentions
/// draw the same value before a vault is even consulted, and a vault layered on
/// top keeps that stable across a whole run.
///
/// A label outside the fake-data catalogue has no believable form, so it falls
/// back to a random UUID token, the same opaque last resort as `RandomToken`,
/// rather than leaking the original.
///
/// [`Generator`]: elide_core::redaction::Generator
#[derive(Debug, Clone)]
pub struct FakeGenerator {
    default_language: LanguageTag,
    seed: u64,
}

impl FakeGenerator {
    /// A generator defaulting to English for entities that carry no language
    /// tag, with an unsalted RNG.
    pub fn new() -> Self {
        Self {
            default_language: LanguageTag::from_str("en").expect("en is BCP-47"),
            seed: 0,
        }
    }

    /// Override the language used when an entity carries no `language` tag.
    /// Initial value is `"en"`.
    #[must_use]
    pub fn with_default_language(mut self, language: LanguageTag) -> Self {
        self.default_language = language;
        self
    }

    /// Salt the per-entity RNG with `seed`. Two generators with the same seed
    /// produce the same fake value for the same entity.
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// The believable value for `entity` reading `original`, or a deterministic
    /// opaque token when the label is outside the fake-data catalogue (still
    /// consistent per entity, so coreferent mentions match).
    fn value(&self, entity: &Entity<impl Modality>, original: &str) -> String {
        fake_value(entity, original, self.seed, &self.default_language)
            .unwrap_or_else(|| fallback_token(entity, self.seed))
    }
}

impl Default for FakeGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator<Text> for FakeGenerator {
    fn generate(&self, entity: &Entity<Text>, data: &TextData) -> TextReplacement {
        TextReplacement::substituted(self.value(entity, data.as_str()))
    }
}

impl Generator<Tabular> for FakeGenerator {
    fn generate(&self, entity: &Entity<Tabular>, data: &TextData) -> TabularReplacement {
        TabularReplacement::Cell(TextReplacement::substituted(
            self.value(entity, data.as_str()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::{EntityCoRef, LabelRef};
    use elide_core::modality::text::TextLocation;
    use uuid::Uuid;

    use super::*;

    fn entity(label: &str, coref: Option<&str>) -> Entity<Text> {
        let e = Entity::custom(LabelRef::new(label), TextLocation::new(0, 5)).build();
        match coref {
            Some(c) => e.with_coref(EntityCoRef::new(c.to_owned())),
            None => e,
        }
    }

    fn surrogate(generator: &FakeGenerator, entity: &Entity<Text>, original: &str) -> String {
        let TextReplacement::Substituted(value) =
            generator.generate(entity, &TextData::new(original.to_owned()))
        else {
            panic!("expected a substituted value");
        };
        value.to_string()
    }

    #[test]
    fn a_person_name_reads_believable_not_a_uuid() {
        let generator = FakeGenerator::new();
        let value = surrogate(&generator, &entity("person_name", None), "Leah Kim");
        // Not a UUID: a UUID parses; a believable name does not.
        assert!(Uuid::parse_str(&value).is_err(), "got a UUID: {value}");
        assert!(!value.is_empty());
    }

    #[test]
    fn an_email_keeps_its_shape() {
        let generator = FakeGenerator::new();
        // Structured labels pattern-preserve the original read from data.
        let value = surrogate(
            &generator,
            &entity("email_address", None),
            "leah.kim@example.com",
        );
        assert!(
            value.contains('@'),
            "an email surrogate stays email-shaped: {value}"
        );
        assert_ne!(value, "leah.kim@example.com", "but is not the original");
    }

    #[test]
    fn coreferent_mentions_draw_the_same_value() {
        let generator = FakeGenerator::new();
        // Two mentions of the same cluster seed the same RNG, so before any vault
        // they already collapse to one surrogate.
        let a = surrogate(&generator, &entity("person_name", Some("alice")), "Leah");
        let b = surrogate(&generator, &entity("person_name", Some("alice")), "Ms. Kim");
        assert_eq!(a, b, "coreferent mentions share a surrogate");
    }

    #[test]
    fn an_unknown_label_falls_back_to_a_token() {
        let generator = FakeGenerator::new();
        // Outside the fake-data catalogue: an opaque UUIDv4 token, never the
        // original.
        let value = surrogate(&generator, &entity("some_custom_label", None), "secret");
        let uuid = Uuid::parse_str(&value).expect("fallback is a UUID token");
        assert_eq!(uuid.get_version_num(), 4, "a valid UUIDv4");
    }

    #[test]
    fn the_fallback_token_is_consistent_per_entity() {
        // The consistency guarantee holds for unsupported labels too: a
        // coreferent mention gets the same opaque token, not a fresh UUID.
        let generator = FakeGenerator::new();
        let a = surrogate(&generator, &entity("some_custom_label", Some("c1")), "one");
        let b = surrogate(&generator, &entity("some_custom_label", Some("c1")), "two");
        assert_eq!(a, b, "coreferent fallback tokens match");
        // A different cluster gets a different token.
        let c = surrogate(
            &generator,
            &entity("some_custom_label", Some("c2")),
            "three",
        );
        assert_ne!(a, c, "distinct clusters get distinct tokens");
    }
}
