//! End-to-end: a pattern match records the rule that found it.

use elide_core::entity::audit::AuditKind;
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_pattern::{PatternRecognizer, patterns};

#[tokio::test]
async fn a_validated_match_records_its_validator() {
    let recognizer = PatternRecognizer::builder()
        .with_pattern(patterns::us::ssn())
        .build()
        .expect("recognizer builds");
    let scope = Scope::new();
    let ctx = Context::new(&scope);
    let subject = Subject::<Text>::new(TextData::new("SSN 123-45-6789".to_owned()));
    let entities = recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("recognize");

    let rule = entities
        .iter()
        .flat_map(|e| e.audit.events())
        .find_map(|event| match &event.kind {
            AuditKind::Pattern(pattern) => Some(&pattern.rule),
            _ => None,
        })
        .expect("a pattern event");
    assert_eq!(rule.name, "ssn");
    assert_eq!(rule.validator.as_deref(), Some("us.ssn"));
    assert!(rule.regex.is_some());
}
