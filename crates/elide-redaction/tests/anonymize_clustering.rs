//! Integration tests for overlap-merge / clustering behaviour on the public
//! [`Anonymizer::anonymize`] path: disjoint entities redact separately,
//! overlapping ones collapse to one redaction over the union span run by the
//! safest operator, transitive chains still coalesce, non-coalescible overlaps
//! stay apart, and boxed/arced trait objects flow in as operators.

mod fixtures;

use elide_core::entity::LabelRef;
use elide_core::entity::provenance::EventKind;
use elide_core::modality::text::Text;
use elide_core::operator::Operator;
use elide_core::recognition::Scope;
use elide_operator::operators::{Erase, Replace};
use elide_redaction::{Anonymizer, Rule};
use fixtures::{TextDoc, entity};

// --- clustering & overlap -------------------------------------------------

/// Disjoint entities each redact separately — the baseline behaviour.
#[tokio::test]
async fn disjoint_entities_redact_separately() {
    let mut doc = TextDoc("alice and bob".to_owned());
    let mut entities = vec![entity("NAME", (0, 5)), entity("NAME", (10, 13))];
    Anonymizer::new()
        .with(Rule::fallback(Replace::default()))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();
    // Both names replaced, the connecting text untouched.
    assert_eq!(doc.0, "[NAME] and [NAME]");
}

/// Overlapping entities collapse to one redaction over the union span, run by
/// the safest (least-leaky) operator. `Erase` (Irrecoverable) beats `Replace`
/// (Partial).
#[tokio::test]
async fn overlap_merges_under_safest_operator() {
    let mut doc = TextDoc("0123456789abc".to_owned());
    // NAME [0,5) → Replace (Partial); SSN [3,12) → Erase (Irrecoverable).
    let mut entities = vec![entity("NAME", (0, 5)), entity("SSN", (3, 12))];
    Anonymizer::new()
        .with(Rule::label(LabelRef::new("NAME"), Replace::default()))
        .with(Rule::label(LabelRef::new("SSN"), Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // One redaction over the union [0,12), by Erase → the bytes are removed
    // (not substituted), leaving only the trailing "c".
    assert_eq!(doc.0, "c");

    // Both entities record a redaction by the winning operator.
    for entity in &entities {
        let redacted = entity.provenance.events.iter().any(|e| {
            matches!(&e.kind, EventKind::Redaction { operator, .. } if operator.name == "erase")
        });
        assert!(redacted, "every member records the erase redaction");
    }
}

/// A transitive chain (A–B overlap, B–C overlap, A–C disjoint) still collapses
/// to one redaction spanning all three.
#[tokio::test]
async fn transitive_overlap_chain_merges() {
    let mut doc = TextDoc("0123456789abcdef".to_owned());
    let mut entities = vec![
        entity("A", (0, 5)),
        entity("B", (4, 9)),
        entity("C", (8, 13)),
    ];
    Anonymizer::new()
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();
    // The chain collapses to one erase over the union [0,13), leaving "def".
    assert_eq!(doc.0, "def");
}

/// Two entities that overlap by byte range but sit on different pages can't
/// coalesce into one span, so they stay separate: each redacts on its own and
/// neither is dropped.
#[tokio::test]
async fn non_coalescible_overlap_stays_separate() {
    let mut doc = TextDoc("0123456789".to_owned());
    // Same range, different page: overlaps() is true (page is ignored) but
    // union() is None, so clustering must keep them apart.
    let mut a = entity("A", (0, 5));
    a.location.page = Some(1);
    let mut b = entity("B", (0, 5));
    b.location.page = Some(2);
    let mut entities = vec![a, b];

    Anonymizer::new()
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // Neither entity is silently dropped — both record a redaction.
    for entity in &entities {
        assert!(
            entity
                .provenance
                .events
                .iter()
                .any(|e| matches!(&e.kind, EventKind::Redaction { .. })),
            "every entity records its own redaction",
        );
    }
}

/// A trait object built dynamically (as a policy layer would from config)
/// flows straight into the builder: `Operator` is implemented for
/// `Box<dyn Operator>` and `Arc<dyn Operator>`, so neither needs unwrapping to
/// a concrete type first.
#[tokio::test]
async fn boxed_and_arced_trait_objects_are_operators() {
    use std::sync::Arc;

    let mut doc = TextDoc("alice bob".to_owned());
    let boxed: Box<dyn Operator<Text>> = Box::new(Replace::default());
    let arced: Arc<dyn Operator<Text>> = Arc::new(Erase);

    let mut entities = vec![entity("NAME", (0, 5)), entity("SECRET", (6, 9))];
    Anonymizer::new()
        .with(Rule::label(LabelRef::new("NAME"), boxed))
        .with(Rule::label(LabelRef::new("SECRET"), arced))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // Both trait-object operators ran: NAME replaced, SECRET erased.
    assert_eq!(doc.0, "[NAME] ");
}
