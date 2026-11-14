//! [`Fake`]: locale-aware [`Operator`] that swaps detected
//! entities for plausible fake values.
//!
//! [`Operator`]: elide_core::redaction::Operator

use std::str::FromStr;

use elide_core::entity::Entity;
use elide_core::modality::text::TextReplacement;
use elide_core::modality::{Modality, TextRedactable};
use elide_core::primitive::LanguageTag;
use elide_core::redaction::{LeakProfile, Operator};
use elide_core::{Component, Result};

/// An operator that replaces an entity with locale-aware fake data.
///
/// Picks a locale from the entity's BCP-47 `language` field, falling
/// back to the `default_language` (English unless overridden) when
/// the entity carries no tag. RNG state is derived per-call from
/// the entity's coreference id (or its UUID when there is none),
/// so coreferent mentions of the same real-world entity collapse to
/// the same fake value within a run.
///
/// Serves every [`TextRedactable`] modality: text, table cells, and metadata
/// fields. Entity labels outside the core PII catalogue delegate to the
/// `fallback` operator passed at construction, which needs to serve only
/// the modality `Fake` runs in; every built-in text operator (`Mask`,
/// `Replace`, `Erase`, `Sha2Hash`, …) serves them all.
///
/// Structured labels (IBAN, payment card, postal code, phone,
/// date-of-birth, etc.) always pattern-preserve the original, the
/// output's length and character-class layout matches the input,
/// only the digits and letters are randomised. Free-form labels
/// (names, addresses, organisations) emit a fresh locale-aware
/// fake whose length doesn't need to match.
///
/// Generic over the fallback operator type, so each `Fake` binds one
/// concrete fallback at construction; pass an `Arc<dyn Operator<…>>` to
/// choose it at runtime.
#[derive(Clone, Debug, Component)]
#[component(name = "fake", version = "1.0.0")]
pub struct Fake<F> {
    fallback: F,
    default_language: LanguageTag,
    seed: u64,
}

impl<F> Fake<F> {
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

    /// Mint a fake value for `entity`, reading `source` to pattern-preserve
    /// structured labels; `None` when the label is outside the catalogue, so the
    /// caller delegates to its fallback.
    fn try_generate(&self, entity: &Entity<impl Modality>, source: &str) -> Option<String> {
        crate::synth::fake_value(entity, source, self.seed, &self.default_language)
    }
}

#[async_trait::async_trait]
impl<M, F> Operator<M> for Fake<F>
where
    M: TextRedactable,
    F: Operator<M>,
{
    fn leak_profile(&self) -> LeakProfile {
        // A faked value leaves only the entity's position and approximate
        // shape; a label it cannot fake takes the fallback, so the pair is
        // only as safe as the leakier of the two.
        LeakProfile::Partial.min(self.fallback.leak_profile())
    }

    async fn anonymize(
        &self,
        entity: &Entity<M>,
        data: &M::Data,
    ) -> Result<Option<M::Replacement>> {
        match self.try_generate(entity, data.as_ref()) {
            Some(value) => Ok(Some(M::Replacement::from(TextReplacement::substituted(
                value,
            )))),
            None => self.fallback.anonymize(entity, data).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use elide::redaction::operators::{Mask, Replace};
    use elide_core::entity::{EntityCoRef, LabelRef};
    use elide_core::modality::tabular::{Tabular, TabularLocation, TabularReplacement};
    use elide_core::modality::text::{Text, TextData, TextLocation};

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
        Entity::<Text>::new(label, TextLocation::inline(0..source.len()))
    }

    fn coref_entity(label: LabelRef, source: &str, coref_id: &str) -> Entity<Text> {
        Entity::<Text>::new(label, TextLocation::inline(0..source.len()))
            .with_coref(EntityCoRef::new(coref_id))
    }

    #[tokio::test]
    async fn unsupported_kind_delegates_to_fallback() {
        // Diagnosis isn't faked, sensitive clinical labels are
        // intentionally excluded, so it falls through to the
        // fallback operator.
        let op = Fake::new(Replace::new("[redacted]"));
        let source = TextData::new("hypertension");
        let entity = entity_over(diagnosis(), source.as_str());
        let out = op.anonymize(&entity, &source).await.unwrap();
        assert_eq!(out, Some(TextReplacement::substituted("[redacted]")));
    }

    /// A label `Fake` cannot fake takes the fallback, so a leakier fallback
    /// sets the profile.
    #[test]
    fn the_leak_profile_includes_the_fallback() {
        use elide::redaction::operators::Keep;

        assert_eq!(
            Operator::<Text>::leak_profile(&Fake::new(Mask::stars())),
            LeakProfile::Partial
        );
        assert_eq!(
            Operator::<Text>::leak_profile(&Fake::new(Keep)),
            LeakProfile::Recoverable
        );
    }

    #[tokio::test]
    async fn fallback_can_be_mask() {
        let op = Fake::new(Mask::stars());
        let source = TextData::new("hypertension");
        let entity = entity_over(diagnosis(), source.as_str());
        let out = op.anonymize(&entity, &source).await.unwrap();
        assert_eq!(out, Some(TextReplacement::substituted("************")));
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
        let entity = Entity::<Text>::new(
            person_name(),
            TextLocation::inline(0..source.as_str().len()),
        )
        .with_language("ja".parse().unwrap());
        let out = op.anonymize(&entity, &source).await.unwrap();
        let Some(TextReplacement::Substituted(value)) = out else {
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
        let Some(TextReplacement::Substituted(value)) = out else {
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
            let Some(TextReplacement::Substituted(value)) = out else {
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
        let entity = Entity::<Tabular>::new(
            person_name(),
            TabularLocation {
                row_index: 0u32,
                column_index: 0u32,
                span: None,
                column_name: None,
                sheet_name: None,
            },
        );
        let out =
            <Fake<Mask> as Operator<Tabular>>::anonymize(&op, &entity, &TextData::new("alice"))
                .await
                .unwrap();
        let Some(TabularReplacement::Cell(TextReplacement::Substituted(value))) = out else {
            panic!("expected Cell(Substituted)");
        };
        assert!(!value.is_empty());
    }
}
