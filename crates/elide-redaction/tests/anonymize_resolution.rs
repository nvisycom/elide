//! Integration tests for how the public [`Anonymizer::anonymize`] path resolves
//! an entity to an operator — by label, tag, predicate, catalog-predicate, and
//! fallback, first-matching-rule-wins — and the provenance/attribution the run
//! records for each redaction.

mod fixtures;

use elide_core::entity::audit::{
    Attribution, AuditKind, CitedAttribution, FreeformAttribution, RuleMatch,
};
use elide_core::entity::{Entity, EntityCoRef, Label, LabelCatalog, LabelRef};
use elide_core::modality::text::Text;
use elide_core::primitive::{Confidence, ConfidenceThreshold};
use elide_core::recognition::Scope;
use elide_operator::operators::{Erase, Keep, Mask, Replace};
use elide_redaction::{Anonymizer, Rule};
use fixtures::{TextDoc, anonymize_one};

#[tokio::test]
async fn anonymize_resolves_label_to_operator_with_fallback() {
    //                          0         1         2
    //                          0123456789012345678901234567
    let mut doc = TextDoc::new("call 555-867-5309 or a@b.com");
    let mut entities = vec![
        Entity::fixture("PHONE_NUMBER", (5, 17)), // "555-867-5309" -> Mask
        Entity::fixture("EMAIL_ADDRESS", (21, 28)), // "a@b.com" -> fallback Erase
    ];

    Anonymizer::<Text>::new()
        .with(Rule::label(
            LabelRef::new("PHONE_NUMBER"),
            Mask::stars().with_keep_suffix(4),
        ))
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // PHONE_NUMBER masked (last 4 kept); EMAIL_ADDRESS fell back to Erase.
    assert_eq!(doc.text(), "call ********5309 or ");
}

#[tokio::test]
async fn anonymize_replace_renders_label_and_value() {
    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("PERSON"),
            Replace::new("<{label}:{value}>"),
        )),
        "name: Alice",
        Entity::fixture("PERSON", (6, 11)), // "Alice"
    )
    .await;
    assert_eq!(out, "name: <PERSON:Alice>");
}

#[tokio::test]
async fn anonymize_replace_threads_coref_through_template() {
    //                          012345678901234567890
    let mut doc = TextDoc::new("Alice told Bob she left");
    // Alice and "she" share a cluster; Bob is his own.
    let alice = EntityCoRef::new("alice");
    let mut entities = vec![
        Entity::fixture("PERSON", (0, 5)).with_coref(alice.clone()), // "Alice"
        Entity::fixture("PERSON", (11, 14)).with_coref(EntityCoRef::new("bob")), // "Bob"
        Entity::fixture("PERSON", (15, 18)).with_coref(alice),       // "she"
    ];

    Anonymizer::<Text>::new()
        .with(Rule::label(
            LabelRef::new("PERSON"),
            Replace::new("[{label}:{coref}]"),
        ))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // Coreferent mentions render to the same token; Bob's is distinct.
    assert_eq!(
        doc.text(),
        "[PERSON:alice] told [PERSON:bob] [PERSON:alice] left"
    );
}

#[tokio::test]
async fn anonymize_replace_coref_empty_when_unset() {
    let out = anonymize_one(
        Anonymizer::<Text>::new().with(Rule::label(
            LabelRef::new("PERSON"),
            Replace::new("[{label}:{coref}]"),
        )),
        "name: Alice",
        Entity::fixture("PERSON", (6, 11)), // "Alice", no coref
    )
    .await;
    // Unset coref expands to empty.
    assert_eq!(out, "name: [PERSON:]");
}

#[tokio::test]
async fn anonymize_skips_unmapped_without_fallback() {
    // No operator for SSN, no fallback -> skipped; the document is untouched.
    let out = anonymize_one(
        Anonymizer::<Text>::new(),
        "123-45-6789",
        Entity::fixture("SSN", (0, 11)),
    )
    .await;
    assert_eq!(out, "123-45-6789");
}

#[tokio::test]
async fn anonymize_predicate_gates_on_confidence() {
    let mut doc = TextDoc::new("call 555-867-5309 or a@b.com");
    let mut entities = vec![
        Entity::fixture_conf("PHONE_NUMBER", (5, 17), Confidence::clamped(0.2)), // weak -> Keep
        Entity::fixture_conf("EMAIL_ADDRESS", (21, 28), Confidence::MAX),        // strong -> Erase
    ];

    // A weak detection is kept verbatim; everything else falls through to the
    // catch-all. Order matters: the predicate rule precedes the fallback.
    let cutoff = ConfidenceThreshold::clamped(0.5);
    Anonymizer::<Text>::new()
        .with(Rule::predicate(
            move |cx| !cutoff.passes(cx.entity.confidence),
            Keep,
        ))
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // Weak phone kept verbatim; strong email erased.
    assert_eq!(doc.text(), "call 555-867-5309 or ");
}

#[tokio::test]
async fn anonymize_selects_by_tag() {
    let mut doc = TextDoc::new("4111111111111111 and bob");
    let mut entities = vec![
        Entity::fixture("payment_card", (0, 16)), // tagged "financial" -> Mask
        Entity::fixture("person_name", (21, 24)), // no financial tag -> fallback Erase
    ];

    // A catalog gives labels their tags; the tag rule then matches any entity
    // whose label carries "financial".
    let mut catalog = LabelCatalog::new();
    catalog.insert(Label::new("payment_card", "payment card").with_tags(["financial", "pci"]));
    catalog.insert(Label::new("person_name", "person name").with_tags(["pii"]));

    Anonymizer::<Text>::new()
        .with_catalog(catalog)
        .with(Rule::tag("financial", Mask::stars()))
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // Financial-tagged card masked; untagged person erased by the fallback.
    assert_eq!(doc.text(), "**************** and ");
}

#[tokio::test]
async fn catalog_predicate_resolves_tags_through_the_catalog() {
    let mut doc = TextDoc::new("4111111111111111 and bob");
    let mut entities = vec![
        Entity::fixture("payment_card", (0, 16)), // financial -> Mask
        Entity::fixture("person_name", (21, 24)), // not financial -> fallback Erase
    ];

    let mut catalog = LabelCatalog::new();
    catalog.insert(Label::new("payment_card", "payment card").with_tags(["financial"]));
    catalog.insert(Label::new("person_name", "person name").with_tags(["pii"]));

    // A catalog-aware predicate resolves the entity's label to its tags — the
    // same source `with_tag` consults, but expressed as a predicate.
    Anonymizer::<Text>::new()
        .with_catalog(catalog)
        .with(Rule::predicate(
            |cx| {
                cx.catalog
                    .get(&cx.entity.label)
                    .is_some_and(|l| l.has_tag("financial"))
            },
            Mask::stars(),
        ))
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    assert_eq!(doc.text(), "**************** and ");
}

#[tokio::test]
async fn anonymize_first_matching_rule_wins() {
    // Two rules match the same entity; the earlier one wins.
    let out = anonymize_one(
        Anonymizer::<Text>::new()
            .with(Rule::label(
                LabelRef::new("EMAIL_ADDRESS"),
                Replace::new("[FIRST]"),
            ))
            .with(Rule::label(
                LabelRef::new("EMAIL_ADDRESS"),
                Replace::new("[SECOND]"),
            )),
        "a@b.com",
        Entity::fixture("EMAIL_ADDRESS", (0, 7)),
    )
    .await;
    assert_eq!(out, "[FIRST]");
}

#[tokio::test]
async fn anonymize_records_redaction_provenance_with_rule_and_attribution() {
    let mut doc = TextDoc::new("a@b.com here");
    let mut entities = vec![Entity::fixture("EMAIL_ADDRESS", (0, 7))];

    Anonymizer::<Text>::new()
        .with(
            Rule::label(LabelRef::new("EMAIL_ADDRESS"), Replace::new("[X]"))
                .because(Attribution::freeform("gdpr-art-17").with_description("right to erasure")),
        )
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    // The entity now carries a Redaction event describing *why* and *how*.
    let redaction = entities[0]
        .audit
        .events()
        .iter()
        .find_map(|e| match &e.kind {
            AuditKind::Redaction(r) => Some((
                r.operator.clone(),
                r.matched_by.clone(),
                r.attribution.clone(),
            )),
            _ => None,
        })
        .expect("a Redaction event was recorded");

    let (operator, matched_by, attribution) = redaction;
    assert_eq!(operator.name, "replace");
    // Automatic why: matched the exact-label rule.
    assert_eq!(matched_by, RuleMatch::Label(LabelRef::new("EMAIL_ADDRESS")));
    // Author why: the attribution the rule carried.
    let attribution = attribution.expect("attribution recorded");
    let Attribution::Freeform(FreeformAttribution { name, description }) = attribution else {
        panic!("expected a freeform attribution");
    };
    assert_eq!(name, "gdpr-art-17");
    assert_eq!(description.as_deref(), Some("right to erasure"));
}

#[tokio::test]
async fn anonymize_records_fallback_rule_with_no_attribution() {
    let mut doc = TextDoc::new("a@b.com");
    let mut entities = vec![Entity::fixture("EMAIL_ADDRESS", (0, 7))];

    // A bare operator via the fallback rule: matched_by is Fallback, no attribution.
    Anonymizer::<Text>::new()
        .with(Rule::fallback(Erase))
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    let (matched_by, attribution) = entities[0]
        .audit
        .events()
        .iter()
        .find_map(|e| match &e.kind {
            AuditKind::Redaction(r) => Some((r.matched_by.clone(), r.attribution.clone())),
            _ => None,
        })
        .expect("a Redaction event was recorded");

    assert_eq!(matched_by, RuleMatch::Fallback);
    assert!(attribution.is_none());
}

#[tokio::test]
async fn because_records_a_freeform_attribution() {
    let mut doc = TextDoc::new("a@b.com");
    let mut entities = vec![Entity::fixture("EMAIL_ADDRESS", (0, 7))];

    // A freeform attribution built from a bare name, no description.
    Anonymizer::<Text>::new()
        .with(
            Rule::label(LabelRef::new("EMAIL_ADDRESS"), Replace::new("[X]"))
                .because(Attribution::freeform("pci-dss-3.4")),
        )
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    let attribution = entities[0]
        .audit
        .events()
        .iter()
        .find_map(|e| match &e.kind {
            AuditKind::Redaction(r) => r.attribution.clone(),
            _ => None,
        })
        .expect("attribution recorded");
    // A bare name builds a freeform attribution with no description.
    let Attribution::Freeform(FreeformAttribution { name, description }) = attribution else {
        panic!("a bare name builds a freeform attribution");
    };
    assert_eq!(name, "pci-dss-3.4");
    assert!(description.is_none());
}

#[tokio::test]
async fn because_records_a_cited_attribution() {
    let mut doc = TextDoc::new("a@b.com");
    let mut entities = vec![Entity::fixture("EMAIL_ADDRESS", (0, 7))];

    Anonymizer::<Text>::new()
        .with(
            Rule::label(LabelRef::new("EMAIL_ADDRESS"), Replace::new("[X]")).because(
                Attribution::cited("GDPR", "Art. 17(1)")
                    .with_rationale("data subject requested erasure"),
            ),
        )
        .anonymize(&mut doc, &mut entities, &Scope::default())
        .await
        .unwrap();

    let attribution = entities[0]
        .audit
        .events()
        .iter()
        .find_map(|e| match &e.kind {
            AuditKind::Redaction(r) => r.attribution.clone(),
            _ => None,
        })
        .expect("attribution recorded");

    let Attribution::Cited(CitedAttribution {
        authority,
        citation,
        rationale,
    }) = attribution
    else {
        panic!("expected a cited attribution");
    };
    assert_eq!(authority, "GDPR");
    assert_eq!(citation, "Art. 17(1)");
    assert_eq!(rationale.as_deref(), Some("data subject requested erasure"));
}
