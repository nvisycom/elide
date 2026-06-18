//! End-to-end anonymizer test: entities are hidden by their per-label
//! operators, reading values through a `DataReader`, with a fallback for
//! unmapped labels.

use elide_core::entity::{Entity, LabelRef};
use elide_core::primitive::Confidence;
use elide_core::provenance::{Event, PatternEvent, Provenance};
use elide::Anonymizer;
use elide::operators::{Mask, Redact, Replace};

mod fixtures;
use fixtures::{Text, TextLocation, TextReplacement, TextSource};

fn entity(label: &str, loc: (usize, usize)) -> Entity<Text> {
    let label = LabelRef::new(label.to_owned());
    let location = TextLocation::new(loc.0, loc.1);
    let confidence = Confidence::MAX;
    let event = Event::pattern(
        "test",
        confidence,
        location.clone(),
        PatternEvent::default(),
    );
    Entity::new(label, location, confidence, Provenance::new(event))
}

#[tokio::test]
async fn anonymize_resolves_label_to_operator_with_fallback() {
    //            0123456789012345678901234567
    let source = TextSource::new("call 555-867-5309 or a@b.com");
    let entities = vec![
        entity("PHONE_NUMBER", (5, 17)),   // "555-867-5309" -> Mask
        entity("EMAIL_ADDRESS", (21, 28)), // "a@b.com" -> fallback Redact
    ];

    let anonymizer = Anonymizer::<Text>::new()
        .with_operator(
            LabelRef::new("PHONE_NUMBER"),
            Mask::stars().with_keep_suffix(4),
        )
        .with_fallback(Redact);

    let items = anonymizer
        .plan(&entities, &source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(items.len(), 2);
    // PHONE_NUMBER masked, last 4 kept.
    assert_eq!(items[0].0, TextLocation::new(5, 17));
    assert_eq!(items[0].1, TextReplacement::substituted("********5309"));
    // EMAIL_ADDRESS fell back to Redact.
    assert_eq!(items[1].1, TextReplacement::Removed);
}

#[tokio::test]
async fn anonymize_replace_renders_label_and_value() {
    let source = TextSource::new("name: Alice");
    let entities = vec![entity("PERSON", (6, 11))]; // "Alice"

    let items = Anonymizer::<Text>::new()
        .with_operator(LabelRef::new("PERSON"), Replace::new("<{label}:{value}>"))
        .plan(&entities, &source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(items[0].1, TextReplacement::substituted("<PERSON:Alice>"));
}

#[tokio::test]
async fn anonymize_skips_unmapped_without_fallback() {
    let source = TextSource::new("123-45-6789");
    let entities = vec![entity("SSN", (0, 11))];
    // No operator for SSN, no fallback -> skipped.
    let redactions = Anonymizer::<Text>::new()
        .plan(&entities, &source)
        .await
        .unwrap();
    assert!(redactions.is_empty());
}
