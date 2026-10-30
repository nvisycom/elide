//! Organization names anchored on a legal-form suffix.

use std::ops::Range;

use elide_core::entity::builtins;
use elide_core::modality::text::{Text, TextData};
use elide_core::primitive::{Confidence, ConfidenceThreshold};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_pattern::PatternRecognizer;
use elide_pattern::patterns::legal_forms;

/// Every `organization_name` the legal-form pattern finds in `text`: its span
/// and confidence, in the order found.
async fn matches(text: &str) -> Vec<(Range<usize>, Confidence)> {
    let recognizer = PatternRecognizer::builder()
        .with_pattern(legal_forms::organization())
        .build()
        .expect("recognizer builds");
    let scope = Scope::new();
    let ctx = Context::<Text>::new(&scope);
    let subject = Subject::new(TextData::new(text.to_owned()));
    recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("recognize")
        .into_iter()
        .filter(|e| e.label == builtins::ORGANIZATION_NAME.to_ref())
        .map(|e| {
            (
                e.location.range().expect("text range").clone(),
                e.confidence,
            )
        })
        .collect()
}

/// The names found in `text`, with confidence. A match inside another is
/// dropped, as same-label reconciliation would merge it into the longer one.
async fn organizations(text: &str) -> Vec<(String, Confidence)> {
    let found = matches(text).await;
    let mut outermost: Vec<(String, Confidence)> = found
        .iter()
        .filter(|(range, _)| {
            !found.iter().any(|(other, _)| {
                other != range && other.start <= range.start && range.end <= other.end
            })
        })
        .map(|(range, confidence)| (text[range.clone()].to_owned(), *confidence))
        .collect();
    outermost.sort_by(|a, b| a.0.cmp(&b.0));
    outermost.dedup_by(|a, b| a.0 == b.0);
    outermost
}

async fn names(text: &str) -> Vec<String> {
    organizations(text)
        .await
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

#[tokio::test]
async fn finds_names_across_jurisdictions() {
    for (text, name) in [
        ("Invoice from Müller Bau GmbH, Berlin", "Müller Bau GmbH"),
        ("Signed by Acme Holdings Ltd on Monday", "Acme Holdings Ltd"),
        ("Tesco PLC reported results", "Tesco PLC"),
        ("Bank of America Corp. said", "Bank of America Corp."),
        ("Acme, Inc. and its affiliates", "Acme, Inc."),
        ("Dupont Frères SARL à Lyon", "Dupont Frères SARL"),
        ("Società Rossi S.p.A. di Milano", "Società Rossi S.p.A."),
        (
            "Umowa z Kowalski Budownictwo sp. z o.o. z siedzibą",
            "Kowalski Budownictwo sp. z o.o.",
        ),
    ] {
        assert_eq!(names(text).await, [name], "in {text:?}");
    }
}

#[tokio::test]
async fn the_longest_suffix_wins() {
    assert_eq!(
        names("Acme GmbH & Co. KG hat").await,
        ["Acme GmbH & Co. KG"]
    );
    assert_eq!(
        names("Goldman Sachs & Co. advised").await,
        ["Goldman Sachs & Co."]
    );
}

#[tokio::test]
async fn a_connective_after_a_suffix_starts_a_new_name() {
    assert_eq!(
        names("Acme GmbH and Beta GmbH signed").await,
        ["Acme GmbH", "Beta GmbH"]
    );
    assert_eq!(
        names("Bank of America Corp. and Beta GmbH signed").await,
        ["Bank of America Corp.", "Beta GmbH"]
    );
}

#[tokio::test]
async fn a_suffix_word_inside_a_name_stays_part_of_it() {
    assert_eq!(
        names("Served on Arkansas Corporation Services, Inc. today").await,
        ["Arkansas Corporation Services, Inc."]
    );
    assert_eq!(
        names("Acme Corporation Services, Inc. and Beta GmbH").await,
        ["Acme Corporation Services, Inc.", "Beta GmbH"]
    );
}

#[tokio::test]
async fn a_dotted_suffix_is_one_match() {
    let text = "Bank of America Corp. said";
    let found = matches(text).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(&text[found[0].0.clone()], "Bank of America Corp.");
}

#[tokio::test]
async fn a_lead_in_word_is_not_part_of_the_name() {
    assert_eq!(names("The Acme GmbH office").await, ["Acme GmbH"]);
    assert_eq!(names("Von Müller AG erhalten").await, ["Müller AG"]);
}

#[tokio::test]
async fn a_lead_in_word_alone_is_not_a_name() {
    assert!(names("The AG said no.").await.is_empty());
    assert!(names("Der KG wurde gekündigt").await.is_empty());
}

#[tokio::test]
async fn a_name_does_not_cross_a_line_or_sentence() {
    assert_eq!(names("Signed\nAcme GmbH").await, ["Acme GmbH"]);
    assert_eq!(
        names("We met in Berlin. Acme GmbH signed.").await,
        ["Acme GmbH"]
    );
}

#[tokio::test]
async fn needs_a_capitalized_name_and_a_whole_suffix() {
    assert!(names("a small gmbh in town").await.is_empty());
    assert!(names("Peter as well as Anna").await.is_empty());
    assert!(names("Acme GmbHx").await.is_empty());
    assert!(names("GmbH").await.is_empty());
}

#[tokio::test]
async fn short_suffixes_score_below_the_threshold_without_context() {
    let found = organizations("the Texas AG filed").await;
    assert_eq!(found.len(), 1);
    assert!(!ConfidenceThreshold::BASELINE.passes(found[0].1));

    let found = organizations("Siemens GmbH filed").await;
    assert!(ConfidenceThreshold::BASELINE.passes(found[0].1));
}
