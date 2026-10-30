//! End-to-end: feed real input through a [`Pattern`] →
//! [`PatternRecognizer`] and verify that confidence is boosted, and a
//! [`Refinement`] step is appended only for matches that had a nearby
//! keyword.
//!
//! [`Refinement`]: elide_core::entity::audit::AuditKind::Refinement

use elide_core::entity::audit::AuditKind;
use elide_core::entity::builtins;
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::Confidence;
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_pattern::context::RuleContext;
use elide_pattern::{Pattern, PatternRecognizer, Variant};

#[tokio::test]
async fn keywords_boost_matches_near_them_only() {
    let variant = Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
        .expect("ssn variant builds")
        .with_score(Confidence::clamped(0.6));
    let regex = Pattern::builder()
        .with_name("ssn")
        .with_labels(vec![builtins::GOVERNMENT_ID.to_ref()])
        .with_context(vec!["ssn".to_owned(), "social security".to_owned()])
        .with_variants(vec![variant])
        .build()
        .expect("ssn regex builds");

    let recognizer = PatternRecognizer::builder()
        .with_pattern(regex)
        .build()
        .expect("recognizer builds");

    // Two SSN-shaped numbers: one near the keyword, one not.
    let text = "First SSN: 123-45-6789. Unrelated number 987-65-4329 elsewhere.";
    let data = TextData::new(text.to_owned());
    let scope = Scope::new();
    let ctx = Context::<Text>::new(&scope);
    let subject = Subject::new(data);
    let entities = recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("recognize");
    assert_eq!(entities.len(), 2, "two SSN matches expected");

    // First match has `SSN:` within the default 5-word prefix/suffix
    // window and gets boosted by the recognizer's context.
    let near = entities
        .iter()
        .find(|e| {
            e.location
                .range()
                .is_some_and(|r| &text[r.start..r.end] == "123-45-6789")
        })
        .expect("near match present");
    assert!(
        near.confidence.get() > 0.6,
        "near-keyword match should be boosted",
    );
    assert!(
        near.audit
            .events()
            .iter()
            .any(|e| matches!(e.kind, AuditKind::Refinement(_))),
        "near-keyword match should have a Refinement step",
    );

    // Second match is well outside the window → untouched.
    let far = entities
        .iter()
        .find(|e| {
            e.location
                .range()
                .is_some_and(|r| &text[r.start..r.end] == "987-65-4329")
        })
        .expect("far match present");
    assert!(
        (far.confidence.get() - 0.6).abs() < f32::EPSILON,
        "far-from-keyword match should not be boosted",
    );
    assert!(
        !far.audit
            .events()
            .iter()
            .any(|e| matches!(e.kind, AuditKind::Refinement(_))),
        "far-from-keyword match should have no Refinement step",
    );
}

/// A `[context]` table may override the default `+0.35` boost. A pattern that
/// sets `boost = 0.1` lifts a near-keyword match by exactly that amount, not
/// the enhancer default.
#[tokio::test]
async fn context_boost_override_applies_the_custom_lift() {
    let variant = Variant::new(r"\b\d{3}-\d{2}-\d{4}\b")
        .expect("ssn variant builds")
        .with_score(Confidence::clamped(0.6));
    // `boost = 0.1` on the context table, weaker than the default 0.35.
    let context = RuleContext {
        keywords: vec!["ssn".to_owned()],
        boost: Some(0.1),
        ..RuleContext::default()
    };
    let regex = Pattern::builder()
        .with_name("ssn")
        .with_labels(vec![builtins::GOVERNMENT_ID.to_ref()])
        .with_context(context)
        .with_variants(vec![variant])
        .build()
        .expect("ssn regex builds");

    let recognizer = PatternRecognizer::builder()
        .with_pattern(regex)
        .build()
        .expect("recognizer builds");

    let text = "SSN: 123-45-6789.";
    let data = TextData::new(text.to_owned());
    let scope = Scope::new();
    let ctx = Context::<Text>::new(&scope);
    let subject = Subject::new(data);
    let entities = recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("recognize");

    assert_eq!(entities.len(), 1, "one SSN match expected");
    // 0.6 base + 0.1 override = 0.7 (not 0.6 + 0.35 = 0.95).
    assert!(
        (entities[0].confidence.get() - 0.7).abs() < f32::EPSILON,
        "custom boost 0.1 should lift 0.6 to 0.7, got {}",
        entities[0].confidence.get(),
    );
}

/// A rule's keywords lift its own matches, not another rule's matches of the
/// same label.
#[tokio::test]
async fn keywords_lift_only_their_own_rules_matches() {
    let rule = |name: &str, regex: &str, context: Vec<String>| {
        Pattern::builder()
            .with_name(name)
            .with_labels(vec![builtins::GOVERNMENT_ID.to_ref()])
            .with_context(context)
            .with_variants(vec![
                Variant::new(regex)
                    .expect("variant builds")
                    .with_score(Confidence::clamped(0.4)),
            ])
            .build()
            .expect("rule builds")
    };
    let recognizer = PatternRecognizer::builder()
        .with_pattern(rule(
            "ssn",
            r"\b\d{3}-\d{2}-\d{4}\b",
            vec!["ssn".to_owned()],
        ))
        .with_pattern(rule("other", r"\b[A-Z]{2}\d{6}\b", Vec::new()))
        .build()
        .expect("recognizer builds");

    let text = "SSN 123-45-6789 AB123456";
    let scope = Scope::new();
    let ctx = Context::<Text>::new(&scope);
    let subject = Subject::new(TextData::new(text.to_owned()));
    let entities = recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("recognize");

    let confidence = |found: &str| {
        entities
            .iter()
            .find(|e| {
                e.location
                    .range()
                    .is_some_and(|r| &text[r.clone()] == found)
            })
            .expect("match present")
            .confidence
            .get()
    };
    assert!(confidence("123-45-6789") > 0.4);
    assert!((confidence("AB123456") - 0.4).abs() < f32::EPSILON);
}
