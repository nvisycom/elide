//! A pattern with several candidate labels emits the first one the request
//! catalog declares, so one rule serves consumers that opted into a
//! fine-grained label and those that only enabled a coarser one.

use elide_core::entity::{Label, LabelCatalog, LabelRef, builtins};
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Recognizer, RecognizerContext, Scope};
use elide_pattern::{PatternRecognizer, Regex, Variant};

/// A pattern matching a street-address-like token, listing the specific label
/// first and the coarse one as a fallback.
fn address_recognizer() -> PatternRecognizer {
    let variant = Variant::new(r"\d+ \w+ St").expect("variant builds");
    let rule = Regex::builder()
        .with_name("street")
        .with_labels(vec![
            builtins::STREET_ADDRESS.to_ref(),
            builtins::ADDRESS.to_ref(),
        ])
        .with_variants(vec![variant])
        .build()
        .expect("rule builds");
    PatternRecognizer::builder()
        .with_pattern(rule)
        .build()
        .expect("recognizer builds")
}

/// The single label emitted for the one match in `"12 Main St"` under `scope`.
async fn emitted_label(recognizer: &PatternRecognizer, scope: &Scope) -> LabelRef {
    let data = TextData::new("12 Main St".to_owned());
    let ctx = RecognizerContext::<Text>::new(scope);
    let entities = recognizer.recognize(&data, &ctx).await.expect("recognize");
    assert_eq!(entities.len(), 1, "one match, one entity");
    entities[0].label.clone()
}

/// A catalog declaring exactly the given built-in labels.
fn catalog_of(labels: &[&LabelRef]) -> LabelCatalog {
    labels
        .iter()
        .map(|l| Label::new(l.as_str(), l.as_str()))
        .collect()
}

#[tokio::test]
async fn empty_catalog_emits_the_first_candidate() {
    let recognizer = address_recognizer();
    // No catalog narrows the label set: the most-specific candidate wins.
    let scope = Scope::new();
    assert_eq!(
        emitted_label(&recognizer, &scope).await,
        builtins::STREET_ADDRESS.to_ref(),
    );
}

#[tokio::test]
async fn catalog_with_the_specific_label_emits_it() {
    let recognizer = address_recognizer();
    let scope = Scope::new().with_catalog(catalog_of(&[&builtins::STREET_ADDRESS.to_ref()]));
    assert_eq!(
        emitted_label(&recognizer, &scope).await,
        builtins::STREET_ADDRESS.to_ref(),
    );
}

#[tokio::test]
async fn catalog_with_only_the_coarse_label_falls_back_to_it() {
    let recognizer = address_recognizer();
    // The consumer enabled only ADDRESS, not STREET_ADDRESS: the rule emits the
    // coarser candidate rather than nothing.
    let scope = Scope::new().with_catalog(catalog_of(&[&builtins::ADDRESS.to_ref()]));
    assert_eq!(
        emitted_label(&recognizer, &scope).await,
        builtins::ADDRESS.to_ref(),
    );
}

#[tokio::test]
async fn catalog_with_neither_candidate_emits_nothing() {
    let recognizer = address_recognizer();
    // A catalog that declares an unrelated label enables none of the rule's
    // candidates, so the match contributes no entity.
    let scope = Scope::new().with_catalog(catalog_of(&[&builtins::EMAIL_ADDRESS.to_ref()]));
    let data = TextData::new("12 Main St".to_owned());
    let ctx = RecognizerContext::<Text>::new(&scope);
    let entities = recognizer.recognize(&data, &ctx).await.expect("recognize");
    assert!(entities.is_empty(), "no enabled candidate, no entity");
}
