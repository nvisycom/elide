//! Integration tests for overlap-merge / clustering behaviour on the public
//! [`Anonymizer::anonymize`] path: disjoint entities redact separately,
//! overlapping ones collapse to one redaction over the union span run by the
//! safest operator, transitive chains still coalesce, non-coalescible overlaps
//! stay apart, and boxed/arced trait objects flow in as operators.

mod fixtures;

use elide_core::entity::audit::AuditKind;
use elide_core::entity::{Entity, LabelRef};
use elide_core::modality::text::Text;
use elide_core::recognition::Scope;
use elide_core::redaction::Operator;
use elide_operator::operators::{Erase, Replace};
use elide_redaction::{Anonymizer, Rule};
use fixtures::TextDoc;

/// Disjoint entities each redact separately, the baseline behaviour.
#[tokio::test]
async fn disjoint_entities_redact_separately() {
    let mut doc = TextDoc::new("alice and bob");
    let mut entities = vec![
        Entity::fixture("NAME", (0, 5)),
        Entity::fixture("NAME", (10, 13)),
    ];
    Anonymizer::new()
        .with(Rule::fallback(Replace::default()))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();
    // Both names replaced, the connecting text untouched.
    assert_eq!(doc.text(), "[NAME] and [NAME]");
}

/// Overlapping entities collapse to one redaction over the union span, run by
/// the safest (least-leaky) operator. `Erase` (Irrecoverable) beats `Replace`
/// (Partial).
#[tokio::test]
async fn overlap_merges_under_safest_operator() {
    let mut doc = TextDoc::new("0123456789abc");
    // NAME [0,5) → Replace (Partial); SSN [3,12) → Erase (Irrecoverable).
    let mut entities = vec![
        Entity::fixture("NAME", (0, 5)),
        Entity::fixture("SSN", (3, 12)),
    ];
    Anonymizer::new()
        .with(Rule::label(LabelRef::new("NAME"), Replace::default()))
        .with(Rule::label(LabelRef::new("SSN"), Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // One redaction over the union [0,12), by Erase → the bytes are removed
    // (not substituted), leaving only the trailing "c".
    assert_eq!(doc.text(), "c");

    // Both entities record a redaction by the winning operator.
    for entity in &entities {
        let redacted = entity
            .audit
            .events()
            .iter()
            .any(|e| matches!(&e.kind, AuditKind::Redaction(r) if r.operator.name == "erase"));
        assert!(redacted, "every member records the erase redaction");
    }
}

/// A transitive chain (A–B overlap, B–C overlap, A–C disjoint) still collapses
/// to one redaction spanning all three.
#[tokio::test]
async fn transitive_overlap_chain_merges() {
    let mut doc = TextDoc::new("0123456789abcdef");
    let mut entities = vec![
        Entity::fixture("A", (0, 5)),
        Entity::fixture("B", (4, 9)),
        Entity::fixture("C", (8, 13)),
    ];
    Anonymizer::new()
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();
    // The chain collapses to one erase over the union [0,13), leaving "def".
    assert_eq!(doc.text(), "def");
}

/// Two entities that overlap by byte range but sit on different pages can't
/// coalesce into one span, so they stay separate: each redacts on its own and
/// neither is dropped.
#[tokio::test]
async fn non_coalescible_overlap_stays_separate() {
    let mut doc = TextDoc::new("0123456789");
    // Same range, different page: overlaps() is true (page is ignored) but
    // union() is None, so clustering must keep them apart.
    let mut a = Entity::fixture("A", (0, 5));
    a.location.page = Some(1);
    let mut b = Entity::fixture("B", (0, 5));
    b.location.page = Some(2);
    let mut entities = vec![a, b];

    Anonymizer::new()
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // The invariant under test is a *clustering* one: non-coalescible entities
    // must not merge into a cluster that then drops a member (see the union
    // `expect` in `redact`). It is asserted on the (page-aware) audit trail ,
    // each entity records its own redaction, not on `doc.text()`: the flat
    // `TextDoc` ignores `page`, so page-local write *placement* is out of scope
    // here and would need a page-partitioned double to check.
    for entity in &entities {
        assert!(
            entity.is_redacted(),
            "every entity records its own redaction"
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

    let mut doc = TextDoc::new("alice bob");
    let boxed: Box<dyn Operator<Text>> = Box::new(Replace::default());
    let arced: Arc<dyn Operator<Text>> = Arc::new(Erase);

    let mut entities = vec![
        Entity::fixture("NAME", (0, 5)),
        Entity::fixture("SECRET", (6, 9)),
    ];
    Anonymizer::new()
        .with(Rule::label(LabelRef::new("NAME"), boxed))
        .with(Rule::label(LabelRef::new("SECRET"), arced))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // Both trait-object operators ran: NAME replaced, SECRET erased.
    assert_eq!(doc.text(), "[NAME] ");
}
