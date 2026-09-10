//! A Luhn-valid card number is detected on its own evidence, without a nearby
//! context keyword: its pattern scores above the acceptance cutoff because the
//! checksum makes a match strong evidence, and the Luhn validator gates out
//! anything that fails the checksum.

use elide_core::entity::{LabelCatalog, builtins};
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::ConfidenceThreshold;
use elide_core::recognition::{Recognizer, RecognizerContext, Scope};
use elide_pattern::PatternRecognizer;

/// Scan `text` with the shipped patterns (no context enhancer) and return every
/// payment-card confidence found.
async fn card_confidences(text: &str) -> Vec<f32> {
    let recognizer = PatternRecognizer::builder()
        .with_builtin_patterns()
        .build()
        .expect("recognizer builds");
    let data = TextData::new(text.to_owned());
    let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
    let ctx = RecognizerContext::<Text>::new(&scope);
    Recognizer::<Text>::recognize(&recognizer, &data, &ctx)
        .await
        .expect("recognize")
        .entities
        .into_iter()
        .filter(|e| e.label == builtins::PAYMENT_CARD.to_ref())
        .map(|e| f32::from(e.confidence))
        .collect()
}

#[tokio::test]
async fn a_luhn_valid_card_clears_baseline_without_context() {
    // Three valid cards, no "card"/"visa" keyword anywhere. Each must clear the
    // acceptance cutoff on the pattern's score alone.
    let text = "4111 1111 1111 1111 zzz 4111 1111 1111 1111 zzz 4111 1111 1111 1111";
    let confidences = card_confidences(text).await;
    assert_eq!(
        confidences.len(),
        3,
        "all three cards detected: {confidences:?}"
    );
    for c in &confidences {
        assert!(
            ConfidenceThreshold::BASELINE.passes(elide_core::primitive::Confidence::clamped(*c)),
            "checksum-valid card confidence {c} should clear BASELINE on its own",
        );
    }
}

#[tokio::test]
async fn a_luhn_invalid_number_is_dropped() {
    // Same shape, but the checksum fails: the validator gates it out entirely.
    let text = "4111 1111 1111 1112";
    assert!(
        card_confidences(text).await.is_empty(),
        "a Luhn-invalid number must not surface as a card",
    );
}
