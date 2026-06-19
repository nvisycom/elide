//! End-to-end anonymizer test: entities are hidden by their per-label
//! operators, reading values through a `DataReader`, with a fallback for
//! unmapped labels.

use elide::Anonymizer;
use elide::redaction::operators::{Erase, Keep, Mask, Replace};
use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
use elide_core::entity::{Entity, EntityCoRef, Label, LabelCatalog, LabelRef};
use elide_core::primitive::{Confidence, ConfidenceThreshold};

mod fixtures;
use fixtures::{Text, TextLocation, TextReplacement, TextSource};

fn entity(label: &str, loc: (usize, usize)) -> Entity<Text> {
    entity_conf(label, loc, Confidence::MAX)
}

fn entity_conf(label: &str, loc: (usize, usize), confidence: Confidence) -> Entity<Text> {
    let label = LabelRef::new(label.to_owned());
    let location = TextLocation::new(loc.0, loc.1);
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
        entity("EMAIL_ADDRESS", (21, 28)), // "a@b.com" -> fallback Erase
    ];

    let anonymizer = Anonymizer::<Text>::new()
        .with_label(
            LabelRef::new("PHONE_NUMBER"),
            Mask::stars().with_keep_suffix(4),
        )
        .with_fallback(Erase);

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
    // EMAIL_ADDRESS fell back to Erase.
    assert_eq!(items[1].1, TextReplacement::Removed);
}

#[tokio::test]
async fn anonymize_replace_renders_label_and_value() {
    let source = TextSource::new("name: Alice");
    let entities = vec![entity("PERSON", (6, 11))]; // "Alice"

    let items = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("PERSON"), Replace::new("<{label}:{value}>"))
        .plan(&entities, &source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(items[0].1, TextReplacement::substituted("<PERSON:Alice>"));
}

#[tokio::test]
async fn anonymize_replace_threads_coref_through_template() {
    //            012345678901234567890
    let source = TextSource::new("Alice told Bob she left");
    // Alice and "she" share a cluster; Bob is his own.
    let alice = EntityCoRef::new("alice");
    let entities = vec![
        entity("PERSON", (0, 5)).with_coref(alice.clone()), // "Alice"
        entity("PERSON", (11, 14)).with_coref(EntityCoRef::new("bob")), // "Bob"
        entity("PERSON", (15, 18)).with_coref(alice),       // "she"
    ];

    let items = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("PERSON"), Replace::new("[{label}:{coref}]"))
        .plan(&entities, &source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();

    // Coreferent mentions render to the same token; Bob's is distinct.
    assert_eq!(items[0].1, TextReplacement::substituted("[PERSON:alice]"));
    assert_eq!(items[2].1, TextReplacement::substituted("[PERSON:alice]"));
    assert_eq!(items[1].1, TextReplacement::substituted("[PERSON:bob]"));
}

#[tokio::test]
async fn anonymize_replace_coref_empty_when_unset() {
    let source = TextSource::new("name: Alice");
    let entities = vec![entity("PERSON", (6, 11))]; // "Alice", no coref

    let items = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("PERSON"), Replace::new("[{label}:{coref}]"))
        .plan(&entities, &source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();

    // Unset coref expands to empty.
    assert_eq!(items[0].1, TextReplacement::substituted("[PERSON:]"));
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

#[tokio::test]
async fn anonymize_predicate_gates_on_confidence() {
    let source = TextSource::new("call 555-867-5309 or a@b.com");
    let entities = vec![
        entity_conf("PHONE_NUMBER", (5, 17), Confidence::clamped(0.2)), // weak -> Keep
        entity_conf("EMAIL_ADDRESS", (21, 28), Confidence::MAX),        // strong -> Erase
    ];

    // A weak detection is kept verbatim; everything else falls through to
    // the catch-all. Order matters: the predicate rule precedes the
    // fallback.
    let cutoff = ConfidenceThreshold::clamped(0.5);
    let items = Anonymizer::<Text>::new()
        .with_predicate(move |e| !cutoff.passes(e.confidence), Keep)
        .with_fallback(Erase)
        .plan(&entities, &source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(items.len(), 2);
    // Weak phone kept verbatim.
    assert_eq!(items[0].1, TextReplacement::substituted("555-867-5309"));
    // Strong email erased.
    assert_eq!(items[1].1, TextReplacement::Removed);
}

#[tokio::test]
async fn anonymize_selects_by_tag() {
    let source = TextSource::new("4111111111111111 and bob");
    let entities = vec![
        entity("payment_card", (0, 16)), // tagged "financial" -> Mask
        entity("person_name", (21, 24)), // no financial tag -> fallback Erase
    ];

    // A catalog gives labels their tags; the tag rule then matches any
    // entity whose label carries "financial".
    let mut catalog = LabelCatalog::new();
    catalog.insert(Label::from_static(
        "payment_card",
        None,
        &["financial", "pci"],
    ));
    catalog.insert(Label::from_static("person_name", None, &["pii"]));

    let items = Anonymizer::<Text>::new()
        .with_catalog(catalog)
        .with_tag("financial", Mask::stars())
        .with_fallback(Erase)
        .plan(&entities, &source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(items.len(), 2);
    // Financial-tagged card masked.
    assert_eq!(items[0].1, TextReplacement::substituted("****************"));
    // Untagged person erased by the fallback.
    assert_eq!(items[1].1, TextReplacement::Removed);
}

#[tokio::test]
async fn anonymize_first_matching_rule_wins() {
    let source = TextSource::new("a@b.com");
    let entities = vec![entity("EMAIL_ADDRESS", (0, 7))];

    // Two rules match the same entity; the earlier one wins.
    let items = Anonymizer::<Text>::new()
        .with_label(LabelRef::new("EMAIL_ADDRESS"), Replace::new("[FIRST]"))
        .with_label(LabelRef::new("EMAIL_ADDRESS"), Replace::new("[SECOND]"))
        .plan(&entities, &source)
        .await
        .unwrap()
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(items[0].1, TextReplacement::substituted("[FIRST]"));
}
