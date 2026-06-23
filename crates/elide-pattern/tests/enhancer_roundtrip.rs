//! End-to-end: feed real input through a [`Regex`] →
//! [`PatternRecognizer`] (wrapped in [`Enhanced`]) and verify
//! that confidence is boosted, and a [`Refinement`] step is
//! appended only for matches that had a nearby keyword.
//!
//! [`Refinement`]: elide_core::entity::provenance::EventKind::Refinement
//! [`Enhanced`]: elide_context::Enhanced

use elide_core::entity::builtins;
use elide_core::entity::provenance::EventKind;
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::Confidence;
use elide_core::recognition::{Recognizer, RecognizerContext, Scope};
use elide_pattern::{PatternRecognizer, Regex, Variant};

#[tokio::test]
async fn enhancer_boosts_matches_near_keyword_only() {
    let variant = Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
        .expect("ssn variant builds")
        .with_score(Confidence::clamped(0.6));
    let regex = Regex::builder()
        .with_name("ssn")
        .with_label(builtins::GOVERNMENT_ID.to_ref())
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
    let data = TextData::new(text.to_owned());
    let scope = Scope::<Text>::new();
    let ctx = RecognizerContext::new(&scope);
    let entities = recognizer.recognize(&data, &ctx).await.expect("recognize");
    assert_eq!(entities.len(), 2, "two SSN matches expected");

    // First match has `SSN:` within the default 5-word prefix/suffix
    // window and gets boosted by the Enhanced<PatternRecognizer> wrapper.
    let near = entities
        .iter()
        .find(|e| &text[e.location.start..e.location.end] == "123-45-6789")
        .expect("near match present");
    assert!(
        near.confidence.get() > 0.6,
        "near-keyword match should be boosted",
    );
    assert!(
        near.provenance
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::Refinement { .. })),
        "near-keyword match should have a Refinement step",
    );

    // Second match is well outside the window → untouched.
    let far = entities
        .iter()
        .find(|e| &text[e.location.start..e.location.end] == "987-65-4329")
        .expect("far match present");
    assert!(
        (far.confidence.get() - 0.6).abs() < f32::EPSILON,
        "far-from-keyword match should not be boosted",
    );
    assert!(
        !far.provenance
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::Refinement { .. })),
        "far-from-keyword match should have no Refinement step",
    );
}
