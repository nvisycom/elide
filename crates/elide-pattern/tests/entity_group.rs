//! A variant's `entity` capture group narrows each match to that group, so a
//! pattern can require surrounding context without reporting it.

use elide_core::entity::{LabelRef, builtins};
use elide_core::modality::text::{Text, TextData};
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_pattern::validators::ValidatorRegistry;
use elide_pattern::{PatternRecognizer, Regex, Variant};

/// One `internal_id` rule with a single variant over `regex`.
fn rule(regex: &str) -> Regex {
    rule_with(Variant::new(regex).expect("regex compiles"))
}

fn rule_with(variant: Variant) -> Regex {
    Regex::builder()
        .with_name("test")
        .with_labels(vec![builtins::INTERNAL_ID.to_ref()])
        .with_variants(vec![variant])
        .build()
        .expect("rule builds")
}

/// The text of every `label` entity `recognizer` finds in `text`.
async fn found(recognizer: &PatternRecognizer, text: &str, label: LabelRef) -> Vec<String> {
    let scope = Scope::new();
    let ctx = Context::<Text>::new(&scope);
    let subject = Subject::new(TextData::new(text.to_owned()));
    recognizer
        .recognize(&subject, &ctx)
        .await
        .expect("recognize")
        .into_iter()
        .filter(|e| e.label == label)
        .map(|e| text[e.location.range().expect("text range").clone()].to_owned())
        .collect()
}

async fn found_by(regex: Regex, text: &str) -> Vec<String> {
    let recognizer = PatternRecognizer::builder()
        .with_pattern(regex)
        .build()
        .expect("recognizer builds");
    found(&recognizer, text, builtins::INTERNAL_ID.to_ref()).await
}

#[tokio::test]
async fn leading_context_is_not_part_of_the_span() {
    let hits = found_by(rule(r"ID:\s*(?P<entity>\d{4})"), "badge ID: 1234, ID:5678").await;
    assert_eq!(hits, ["1234", "5678"]);
}

#[tokio::test]
async fn trailing_context_is_required_but_not_reported() {
    let hits = found_by(rule(r"(?P<entity>\b\d{4})(?:$|[^\w-])"), "1234 5678-x 9012").await;
    assert_eq!(hits, ["1234", "9012"]);
}

#[tokio::test]
async fn an_optional_group_that_did_not_take_part_yields_nothing() {
    let hits = found_by(rule(r"\bref(?:=(?P<entity>\d+))?"), "ref ref=42").await;
    assert_eq!(hits, ["42"]);
}

#[tokio::test]
async fn a_regex_without_the_group_reports_the_whole_match() {
    let hits = found_by(rule(r"ID:\s*(\d{4})"), "badge ID: 1234").await;
    assert_eq!(hits, ["ID: 1234"]);
}

#[tokio::test]
async fn the_validator_sees_only_the_group() {
    let validators = ValidatorRegistry::builtin().with_simple("four-digits", |s: &str| {
        s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit())
    });
    let regex = rule_with(
        Variant::new(r"ID:\s*(?P<entity>\d+)")
            .expect("regex compiles")
            .with_validator("four-digits"),
    );
    let recognizer = PatternRecognizer::builder()
        .with_pattern(regex)
        .with_validators(validators)
        .build()
        .expect("recognizer builds");
    let hits = found(
        &recognizer,
        "ID: 1234 and ID: 123456",
        builtins::INTERNAL_ID.to_ref(),
    )
    .await;
    assert_eq!(hits, ["1234"]);
}

#[tokio::test]
async fn ipv6_matches_whole_tokens_only() {
    let recognizer = PatternRecognizer::builder()
        .with_builtin_patterns()
        .build()
        .expect("recognizer builds");
    let ip = |text: &'static str| {
        let recognizer = &recognizer;
        async move { found(recognizer, text, builtins::IP_ADDRESS.to_ref()).await }
    };

    // Not the valid prefix of a malformed token.
    assert!(ip("bad 2001:db8::1g here").await.is_empty());
    assert!(ip("bad 2001:db8::1.5 here").await.is_empty());
    assert_eq!(ip("at 2001:db8::1.").await, ["2001:db8::1"]);
    assert_eq!(
        ip("(2001:db8::8a2e:370:7334)").await,
        ["2001:db8::8a2e:370:7334"]
    );
    assert_eq!(ip("[2001:db8::1]:443").await, ["2001:db8::1"]);
    assert_eq!(ip("net 2001:db8::/32, ok").await, ["2001:db8::/32"]);
    assert_eq!(ip("link fe80::1%eth0 up").await, ["fe80::1%eth0"]);
    // The IPv4 pattern also finds the embedded quad; reconciliation merges them.
    assert!(
        ip("mapped ::ffff:192.0.2.1 here")
            .await
            .contains(&"::ffff:192.0.2.1".to_owned())
    );
    assert_eq!(ip("loopback ::1").await, ["::1"]);
}
