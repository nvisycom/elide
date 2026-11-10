//! Flagged HTML hidden content through the public facade: a `<meta>` field and
//! a comment under the dispositions and exclusions that decide its fate.

use std::ops::Range;

use elide::codec::FormatRegistry;
use elide::entity::LabelCatalog;
use elide::modality::text::{Text, TextLocation};
use elide::recognition::Scope;
use elide::recognition::annotation::{Annotations, Exclusion};
use elide::redaction::{Disposition, HiddenContentPolicy};
use elide::{Directives, Document, Orchestrator};

/// `raw` decoded under `policy`, run under `scope` and `directives`, re-encoded.
async fn redact(
    raw: &str,
    policy: HiddenContentPolicy,
    scope: Scope,
    directives: Directives,
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
    orchestrator
        .anonymize(&mut documents, &directives)
        .await
        .expect("anonymize");
    let out = documents[0].document.encode().expect("encode").to_bytes();
    String::from_utf8(out.to_vec()).unwrap()
}

/// A scanned `<meta>` field is text recognizers read, so a declared catalog does
/// not erase it without a finding.
#[tokio::test]
async fn scan_as_content_keeps_a_meta_field_nothing_detects() {
    let raw =
        r#"<html><head><meta name="description" content="A page"></head><body>x</body></html>"#;
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::ScanAsContent);
    let scope = Scope::new().with_catalog(LabelCatalog::with_builtins());
    let out = redact(raw, policy, scope, Directives::new()).await;
    assert!(out.contains("A page"), "{out}");
}

#[tokio::test]
async fn flag_everything_erases_a_meta_field_with_no_catalog() {
    let raw =
        r#"<html><head><meta name="description" content="A page"></head><body>x</body></html>"#;
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
    let out = redact(raw, policy, Scope::new(), Directives::new()).await;
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
            let out = redact(&raw, policy, Scope::new(), Directives::new()).await;
            assert!(!out.contains("Jane Roe"), "{policy:?}: {out}");
        }
    }
}

#[tokio::test]
async fn every_repeated_content_of_a_field_is_erased() {
    let raw = r#"<html><head><meta name="author" content="Bob" content="Jane Roe"></head><body>x</body></html>"#;
    let policy = HiddenContentPolicy::default().with_metadata(Disposition::FlagEverything);
    let out = redact(raw, policy, Scope::new(), Directives::new()).await;
    assert!(!out.contains("Bob"), "{out}");
    assert!(!out.contains("Jane Roe"), "{out}");
}

/// A comment redacted under an exclusion of `range` in the decoded text.
async fn comment_excluding(range: Range<usize>) -> String {
    let raw = "<!-- Acme call Jane --><p>x</p>";
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::FlagEverything);
    let annotations =
        Annotations::<Text>::new().with_exclusion(Exclusion::new(TextLocation::inline(range)));
    let directives = Directives::new().with_annotations(annotations);
    redact(raw, policy, Scope::new(), directives).await
}

#[tokio::test]
async fn an_exclusion_inside_a_flagged_item_does_not_spare_the_rest() {
    let out = comment_excluding(1..5).await;
    assert!(!out.contains("Jane"), "{out}");
}

#[tokio::test]
async fn an_exclusion_of_a_whole_flagged_item_spares_it() {
    let out = comment_excluding(0..16).await;
    assert!(out.contains("Acme call Jane"), "{out}");
}
