//! End-to-end: feed real input through a [`Regex`] →
//! [`PatternRecognizer`] (wrapped in [`ContextEnhanced`]) and verify
//! that confidence is boosted, and a [`Refinement`] step is
//! appended only for matches that had a nearby keyword.
//!
//! [`Refinement`]: nvisy_core::entity::TrailStepKind::Refinement
//! [`ContextEnhanced`]: nvisy_context::ContextEnhanced

use nvisy_core::entity::{TrailStepKind, builtins};
use nvisy_core::modality::TextData;
use nvisy_core::primitive::Confidence;
use nvisy_core::recognition::{EntityRecognizer, RecognizerInput};
use nvisy_pattern::{PatternRecognizer, Regex, Variant};

#[tokio::test]
async fn enhancer_boosts_matches_near_keyword_only() {
    let variant = Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
        .expect("ssn variant builds")
        .with_score(Confidence::clamped(0.6));
    let regex = Regex::builder()
        .with_name("ssn")
        .with_label(builtins::GOVERNMENT_ID.label_ref())
        .with_context(vec!["ssn".to_owned(), "social security".to_owned()])
        .with_variants(vec![variant])
        .build()
        .expect("ssn regex builds");

    let recognizer = PatternRecognizer::builder()
        .with_pattern(regex)
        .build_context_enhanced()
        .expect("recognizer builds");

    // Two SSN-shaped numbers: one near the keyword, one not.
    let text = "First SSN: 123-45-6789. Unrelated number 987-65-4329 elsewhere.";
    let input = RecognizerInput::new(TextData::new(text.to_owned()));
    let entities = recognizer
        .recognize(&input)
        .await
        .expect("recognize")
        .entities;
    assert_eq!(entities.len(), 2, "two SSN matches expected");

    // First match has `SSN:` within the default 5-word prefix/suffix
    // window and gets boosted by the ContextEnhanced<PatternRecognizer> wrapper.
    let near = entities
        .iter()
        .find(|e| &text[e.location.start..e.location.end] == "123-45-6789")
        .expect("near match present");
    assert!(
        near.confidence.get() > 0.6,
        "near-keyword match should be boosted",
    );
    assert!(
        near.trail
            .iter()
            .any(|s| matches!(s.kind, TrailStepKind::Refinement)),
        "near-keyword match should have a Refinement step",
    );

    // Second match is well outside the window → untouched.
    let far = entities
        .iter()
        .find(|e| &text[e.location.start..e.location.end] == "987-65-4329")
        .expect("far match present");
    assert!(
        (far.confidence.get() - 0.6).abs() < f64::EPSILON,
        "far-from-keyword match should not be boosted",
    );
    assert!(
        !far.trail
            .iter()
            .any(|s| matches!(s.kind, TrailStepKind::Refinement)),
        "far-from-keyword match should have no Refinement step",
    );
}
