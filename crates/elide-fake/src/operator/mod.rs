//! [`Fake`]: locale-aware [`Operator`] that swaps detected
//! entities for plausible fake values.
//!
//! [`Operator`]: elide_core::redaction::Operator

mod identity;

use std::hash::{DefaultHasher, Hash, Hasher};
use std::str::FromStr;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::primitive::LanguageTag;
use elide_core::redaction::{LeakProfile, Operator, OperatorId};
use fake::rand::SeedableRng;
use fake::rand::rngs::SmallRng;

use crate::generator;
use crate::locale::Locale;

use self::identity::Identity;

/// Locale-aware fake-data operator.
///
/// Picks a locale from the entity's BCP-47 `language` field, falling
/// back to the `default_language` (English unless overridden) when
/// the entity carries no tag. RNG state is derived per-call from
/// the entity's coreference id (or its UUID when there is none),
/// so coreferent mentions of the same real-world entity collapse to
/// the same fake value within a run.
///
/// Entity labels outside the core PII catalogue delegate to the
/// `fallback` operator passed at construction. The fallback's
/// modality bound matches whichever `Operator<M>` impl is invoked
/// — call `Fake` from a [`Text`] context and the fallback only
/// needs `Operator<Text>`; call it from a [`Tabular`] context and
/// it only needs `Operator<Tabular>`. Every built-in elide
/// operator (`Mask`, `Replace`, `Erase`, `Hash`) implements both.
///
/// Structured labels (IBAN, payment card, postal code, phone,
/// date-of-birth, etc.) always pattern-preserve the original — the
/// output's length and character-class layout matches the input,
/// only the digits and letters are randomised. Free-form labels
/// (names, addresses, organisations) emit a fresh locale-aware
/// fake whose length doesn't need to match.
///
/// Generic over the fallback operator type because
/// [`elide_core::redaction::Operator`] is not dyn-compatible (its
/// `anonymize` method returns `impl Future`), so a stored
/// `Arc<dyn Operator<…>>` is not available. Each `Fake` instance
/// binds one concrete fallback type at construction.
#[derive(Clone, Debug)]
pub struct Fake<F> {
    fallback: F,
    default_language: LanguageTag,
    seed: u64,
}

impl<F> Fake<F> {
    /// Operator id stamped on every redaction event.
    fn op_id() -> OperatorId {
        OperatorId::new("fake", "1.0.0")
    }

    /// Build a `Fake` operator with `fallback` as the operator
    /// used for entity labels outside the core PII catalogue.
    pub fn new(fallback: F) -> Self {
        Self {
            fallback,
            default_language: LanguageTag::from_str("en").expect("en is BCP-47"),
            seed: 0,
        }
    }

    /// Override the default language used when the entity carries
    /// no `language` tag. Initial value is `"en"`.
    #[must_use]
    pub fn with_default_language(mut self, language: LanguageTag) -> Self {
        self.default_language = language;
        self
    }

    /// Salt the per-call RNG with `seed`. Two operators with the
    /// same seed produce the same fake value for the same entity.
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    fn locale_for(&self, language: Option<&LanguageTag>) -> Locale {
        Locale::from_tag(language.unwrap_or(&self.default_language))
    }

    fn rng_for(&self, identity: Identity<'_>) -> SmallRng {
        let mut hasher = DefaultHasher::new();
        self.seed.hash(&mut hasher);
        identity.hash(&mut hasher);
        SmallRng::seed_from_u64(hasher.finish())
    }

    /// Try the generator for `label`; return `None` if it has no
    /// entry, so the caller can delegate to the fallback.
    fn try_generate(
        &self,
        locale: Locale,
        label: &str,
        identity: Identity<'_>,
        source: &str,
    ) -> Option<String> {
        let mut rng = self.rng_for(identity);
        generator::Context::new(locale, label, source).generate(&mut rng)
    }
}

impl<F> Operator<Text> for Fake<F>
where
    F: Operator<Text>,
{
    fn id(&self) -> OperatorId {
        Self::op_id()
    }

    fn leak_profile(&self) -> LeakProfile {
        // The original value is gone; only the entity's position
        // and approximate shape (length differs from the original)
        // leak.
        LeakProfile::Partial
    }

    async fn anonymize(&self, entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        let locale = self.locale_for(entity.language.as_ref());
        match self.try_generate(
            locale,
            entity.label.as_str(),
            Identity::from(entity),
            data.as_str(),
        ) {
            Some(value) => Ok(TextReplacement::substituted(value)),
            None => self.fallback.anonymize(entity, data).await,
        }
    }
}

impl<F> Operator<Tabular> for Fake<F>
where
    F: Operator<Tabular>,
{
    fn id(&self) -> OperatorId {
        Self::op_id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Partial
    }

    async fn anonymize(
        &self,
        entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TabularReplacement> {
        let locale = self.locale_for(entity.language.as_ref());
        if let Some(value) = self.try_generate(
            locale,
            entity.label.as_str(),
            Identity::from(entity),
            data.as_str(),
        ) {
            return Ok(TabularReplacement::Cell(TextReplacement::substituted(value)));
        }
        self.fallback.anonymize(entity, data).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use elide::redaction::operators::{Mask, Replace};
    use elide_core::entity::{EntityCoRef, LabelRef};
    use elide_core::modality::tabular::TabularLocation;
    use elide_core::modality::text::TextLocation;
    use elide_core::primitive::Confidence;

    use super::*;

    /// Built-in label refs the tests assert against. Names mirror
    /// the old `nvisy_core::entity::builtins::*` constants so
    /// behavioural tests remain readable.
    fn person_name() -> LabelRef {
        LabelRef::new("person_name")
    }

    fn diagnosis() -> LabelRef {
        LabelRef::new("diagnosis")
    }

    fn fake() -> Fake<Mask> {
        Fake::new(Mask::stars())
    }

    /// Build a Text entity over the full span of `source` so
    /// offsets are self-documenting.
    fn entity_over(label: LabelRef, source: &str) -> Entity<Text> {
        Entity::<Text>::builder()
            .with_label(label)
            .with_location(TextLocation::new(0, source.len()))
            .with_confidence(Confidence::clamped(1.0))
            .build()
            .expect("text entity has label + location + confidence")
    }

    fn coref_entity(label: LabelRef, source: &str, coref_id: &str) -> Entity<Text> {
        Entity::<Text>::builder()
            .with_label(label)
            .with_location(TextLocation::new(0, source.len()))
            .with_confidence(Confidence::clamped(1.0))
            .with_coref(EntityCoRef::new(coref_id))
            .build()
            .expect("text entity has label + location + confidence")
    }

    #[tokio::test]
    async fn unsupported_kind_delegates_to_fallback() {
        // Diagnosis isn't faked — sensitive clinical labels are
        // intentionally excluded — so it falls through to the
        // fallback operator.
        let op = Fake::new(Replace::new("[redacted]"));
        let source = TextData::new("hypertension");
        let entity = entity_over(diagnosis(), source.as_str());
        let out = op.anonymize(&entity, &source).await.unwrap();
        assert_eq!(out, TextReplacement::substituted("[redacted]"));
    }

    #[tokio::test]
    async fn fallback_can_be_mask() {
        let op = Fake::new(Mask::stars());
        let source = TextData::new("hypertension");
        let entity = entity_over(diagnosis(), source.as_str());
        let out = op.anonymize(&entity, &source).await.unwrap();
        assert_eq!(out, TextReplacement::substituted("************"));
    }

    #[tokio::test]
    async fn same_seed_and_entity_id_produces_same_output() {
        let op = fake().with_seed(42);
        let source = TextData::new("alice");
        let entity = entity_over(person_name(), source.as_str());
        let a = op.anonymize(&entity, &source).await.unwrap();
        let b = op.anonymize(&entity, &source).await.unwrap();
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn entity_language_overrides_default() {
        let op = fake();
        let source = TextData::new("名前");
        let entity = Entity::<Text>::builder()
            .with_label(person_name())
            .with_location(TextLocation::new(0, source.as_str().len()))
            .with_confidence(Confidence::clamped(1.0))
            .with_language("ja".parse().unwrap())
            .build()
            .unwrap();
        let out = op.anonymize(&entity, &source).await.unwrap();
        let TextReplacement::Substituted(value) = out else {
            panic!("expected Substituted variant");
        };
        assert!(!value.is_empty(), "ja name should not be empty");
    }

    #[tokio::test]
    async fn default_language_applies_when_entity_unlanguaged() {
        let op = fake().with_default_language("ja".parse().unwrap());
        let source = TextData::new("name");
        let entity = entity_over(person_name(), source.as_str());
        let out = op.anonymize(&entity, &source).await.unwrap();
        let TextReplacement::Substituted(value) = out else {
            panic!("expected Substituted variant");
        };
        assert!(!value.is_empty());
    }

    #[tokio::test]
    async fn coreferent_entities_collapse_to_same_fake() {
        let op = fake();
        let source = TextData::new("alice");
        let a = coref_entity(person_name(), source.as_str(), "ENTITY_42");
        let b = coref_entity(person_name(), source.as_str(), "ENTITY_42");
        let out_a = op.anonymize(&a, &source).await.unwrap();
        let out_b = op.anonymize(&b, &source).await.unwrap();
        assert_eq!(out_a, out_b);
        let c = coref_entity(person_name(), source.as_str(), "ENTITY_99");
        let out_c = op.anonymize(&c, &source).await.unwrap();
        assert_ne!(
            out_a, out_c,
            "different coref id should yield different fake"
        );
    }

    #[tokio::test]
    async fn distinct_entities_get_distinct_fakes() {
        let op = fake();
        let source = TextData::new("seed");
        let mut outputs: HashSet<String> = HashSet::new();
        for _ in 0..32 {
            let entity = entity_over(person_name(), source.as_str());
            let out = op.anonymize(&entity, &source).await.unwrap();
            let TextReplacement::Substituted(value) = out else {
                panic!("expected Substituted");
            };
            outputs.insert(value);
        }
        assert_eq!(
            outputs.len(),
            32,
            "expected 32 distinct fakes across 32 fresh entity ids"
        );
    }

    #[tokio::test]
    async fn tabular_impl_emits_cell_substituted() {
        let op = fake();
        let entity = Entity::<Tabular>::builder()
            .with_label(person_name())
            .with_location(TabularLocation {
                row_index: 0u32,
                column_index: 0u32,
                start_offset: None,
                end_offset: None,
                column_name: None,
                sheet_name: None,
            })
            .with_confidence(Confidence::clamped(1.0))
            .build()
            .unwrap();
        let out = <Fake<Mask> as Operator<Tabular>>::anonymize(
            &op,
            &entity,
            &TextData::new("alice"),
        )
        .await
        .unwrap();
        let TabularReplacement::Cell(TextReplacement::Substituted(value)) = out else {
            panic!("expected Cell(Substituted)");
        };
        assert!(!value.is_empty());
    }
}
