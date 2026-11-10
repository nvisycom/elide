//! Flagged HTML hidden content through the public facade: a `<meta>` field and
//! a comment under the dispositions, and the review edits, that decide its fate.

use std::ops::Range;

use elide::codec::FormatRegistry;
use elide::entity::LabelCatalog;
use elide::modality::text::{Text, TextLocation};
use elide::recognition::Scope;
use elide::redaction::{Disposition, HiddenContentPolicy};
use elide::{Document, Orchestrator};

/// `raw` decoded under `policy`, run under `scope`, with each part of the
/// report's document the `region` contains suppressed, re-encoded.
async fn redact_sparing(
    raw: &str,
    policy: HiddenContentPolicy,
    scope: Scope,
    region: Option<Range<usize>>,
) -> String {
    let registry = FormatRegistry::with_builtin();
    let orchestrator = Orchestrator::new()
        .with_scope(scope)
        .with_registry(registry.clone());
    let bytes = bytes::Bytes::from(raw.as_bytes().to_vec());
    let handle = registry
        .decode(bytes, "html", policy)
        .await
        .expect("decode html");
    let mut documents = [Document::new("page.html", handle)];
    let mut report = orchestrator
        .analyze(&mut documents)
        .await
        .expect("analyze")
        .report;
    if let Some(region) = region {
        report
            .document_mut::<Text>()
            .expect("the page is text")
            .suppress_within(&TextLocation::inline(region));
    }
    orchestrator
        .anonymize_with(&mut documents, report)
        .await
        .expect("anonymize");
    let out = documents[0].document.encode().expect("encode").to_bytes();
    String::from_utf8(out.to_vec()).unwrap()
}

/// `raw` decoded under `policy`, run under `scope`, re-encoded.
async fn redact(raw: &str, policy: HiddenContentPolicy, scope: Scope) -> String {
    redact_sparing(raw, policy, scope, None).await
}

/// A scanned `<meta>` field is text recognizers read, so a declared catalog does
/// not erase it without a finding.
#[tokio::test]
async fn scan_as_content_keeps_a_meta_field_nothing_detects() {
    let raw =
        r#"<html><head><meta name="description" content="A page"></head><body>x</body></html>"#;
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
    let out = redact(raw, policy, scope).await;
    assert!(out.contains("A page"), "{out}");
}

#[tokio::test]
async fn flag_everything_erases_a_meta_field_with_no_catalog() {
    let raw =
        r#"<html><head><meta name="description" content="A page"></head><body>x</body></html>"#;
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
    let out = redact(raw, policy, Scope::new()).await;
    assert!(!out.contains("A page"), "{out}");
    assert!(out.contains("<body>x</body>"), "{out}");
}

/// Tags a browser reads as a rendering tag or an empty field, with a value it
/// ignores still in the document.
const REPEATED: [&str; 7] = [
    r#"<meta name="" name="viewport" content="Jane Roe">"#,
    r#"<meta name name="viewport" content="Jane Roe">"#,
    r#"<meta http-equiv="" http-equiv="content-type" content="Jane Roe">"#,
    r#"<meta name="author" content="Bob" content="Jane Roe">"#,
    r#"<meta name="author" content="" content="Jane Roe">"#,
    r#"<meta charset="utf-8" content="" content="Jane Roe">"#,
    r#"<meta name="viewport" name="author" content="Jane Roe">"#,
];

#[tokio::test]
async fn a_repeated_meta_attribute_hides_nothing() {
    let flag = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
    for tag in REPEATED {
        let raw = format!("<html><head>{tag}</head><body>x</body></html>");
        for policy in [flag, HiddenContentPolicy::default()] {
            let out = redact(&raw, policy, Scope::new()).await;
            assert!(!out.contains("Jane Roe"), "{policy:?}: {out}");
        }
    }
}

#[tokio::test]
async fn every_repeated_content_of_a_field_is_erased() {
    let raw = r#"<html><head><meta name="author" content="Bob" content="Jane Roe"></head><body>x</body></html>"#;
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
    let out = redact(raw, policy, Scope::new()).await;
    assert!(!out.contains("Bob"), "{out}");
    assert!(!out.contains("Jane Roe"), "{out}");
}

/// A comment redacted with the entities `range` contains in the decoded text
/// suppressed.
async fn comment_sparing(range: Range<usize>) -> String {
    let raw = "<!-- Acme call Jane --><p>x</p>";
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::FlagEverything);
    redact_sparing(raw, policy, Scope::new(), Some(range)).await
}

#[tokio::test]
async fn a_region_inside_a_flagged_item_does_not_spare_it() {
    let out = comment_sparing(1..5).await;
    assert!(!out.contains("Jane"), "{out}");
}

#[tokio::test]
async fn a_region_holding_a_whole_flagged_item_spares_it() {
    let out = comment_sparing(0..16).await;
    assert!(out.contains("Acme call Jane"), "{out}");
}
