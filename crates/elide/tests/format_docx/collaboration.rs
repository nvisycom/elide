//! End-to-end through the public facade: a DOCX's collaboration content (a
//! comment, its author, a tracked deletion) under each disposition.
//!
//! With no recognizer registered, nothing in the text is detected, so what is
//! redacted is exactly what the hidden-content policy flags: under
//! `FlagEverything` every comment, author, and deleted passage, erased with no
//! anonymizer rule needed, and under `ScanAsContent` nothing.

use elide::codec::FormatRegistry;
use elide::detection::Analyzer;
use elide::entity::builtins;
use elide::modality::text::Text;
use elide::redaction::operators::Keep;
use elide::redaction::{Anonymizer, Disposition, HiddenContentPolicy, Rule};
use elide::{Document, Orchestrator};
use elide_office::fixtures::{pack_parts, read_part};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// A Word document whose body has visible text, a tracked insertion, and a
/// tracked deletion, with a comment beside it.
fn docx() -> bytes::Bytes {
    let types = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let document_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#;
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="{W}"><w:body><!-- private note --><w:p><w:r><w:t>Payment due in </w:t></w:r><w:del w:id="1" w:author="Jane Roe"><w:r><w:delText>thirty</w:delText></w:r></w:del><w:ins w:id="2" w:author="Jane Roe"><w:r><w:t>sixty</w:t></w:r></w:ins><w:r><w:t> days.</w:t></w:r></w:p></w:body></w:document>"#
    );
    let comments = format!(
        r#"<?xml version="1.0"?><w:comments xmlns:w="{W}"><w:comment w:id="0" w:author="John Doe" w:initials="JD"><w:p><w:r><w:t>Check with legal</w:t></w:r></w:p></w:comment></w:comments>"#
    );
    let styles =
        format!(r#"<?xml version="1.0"?><w:styles xmlns:w="{W}"><!-- private note --></w:styles>"#);
    bytes::Bytes::from(pack_parts(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/_rels/document.xml.rels", document_rels.as_bytes()),
        ("word/document.xml", body.as_bytes()),
        ("word/comments.xml", comments.as_bytes()),
        ("word/styles.xml", styles.as_bytes()),
    ]))
}

/// Decode the fixture with its collaboration content under `collaboration`,
/// redact it with `anonymizer`, and return the encoded package.
async fn encoded(collaboration: Disposition, anonymizer: Anonymizer<Text>) -> Vec<u8> {
    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_collaboration(collaboration);
    let orchestrator = Orchestrator::new()
        .with_registry(registry.clone())
        .with_modality::<Text>(Analyzer::new(), anonymizer);
    let handle = registry
        .decode(docx(), "docx", policy)
        .await
        .expect("decode docx");
    let mut documents = [Document::new("review.docx", handle)];
    orchestrator
        .anonymize(&mut documents)
        .await
        .expect("anonymize");
    documents[0]
        .document
        .encode()
        .expect("encode")
        .to_bytes()
        .to_vec()
}

/// The part `name` of the package `out`, as text.
fn part(out: &[u8], name: &str) -> String {
    String::from_utf8(read_part(out, name).expect(name)).unwrap()
}

/// The body and comments parts of the fixture redacted under `collaboration`
/// with `anonymizer`.
async fn redact(collaboration: Disposition, anonymizer: Anonymizer<Text>) -> (String, String) {
    let out = encoded(collaboration, anonymizer).await;
    (
        part(&out, "word/document.xml"),
        part(&out, "word/comments.xml"),
    )
}

#[tokio::test]
async fn flag_everything_redacts_every_comment_author_and_deletion() {
    let (body, comments) = redact(Disposition::FlagEverything, Anonymizer::new()).await;
    for flagged in ["Check with legal", "John Doe", "\"JD\""] {
        assert!(!comments.contains(flagged), "{flagged} in {comments}");
    }
    for flagged in ["thirty", "Jane Roe"] {
        assert!(!body.contains(flagged), "{flagged} in {body}");
    }
    for visible in ["Payment due in ", "sixty", " days."] {
        assert!(body.contains(visible), "{visible} missing from {body}");
    }
}

#[tokio::test]
async fn scan_as_content_flags_nothing() {
    let (body, comments) = redact(Disposition::ScanAsContent, Anonymizer::new()).await;
    for kept in ["Check with legal", "John Doe"] {
        assert!(comments.contains(kept), "{kept} missing from {comments}");
    }
    for kept in ["thirty", "Jane Roe"] {
        assert!(body.contains(kept), "{kept} missing from {body}");
    }
}

#[tokio::test]
async fn a_rule_keeping_a_flagged_label_wins() {
    let keep_comments =
        Anonymizer::new().with(Rule::label(builtins::COMMUNICATIONS_CONTENT.to_ref(), Keep));
    let (body, comments) = redact(Disposition::FlagEverything, keep_comments).await;
    assert!(comments.contains("Check with legal"), "{comments}");
    assert!(!comments.contains("John Doe"), "{comments}");
    assert!(!body.contains("Jane Roe"), "{body}");
}

#[tokio::test]
async fn xml_comments_go_under_every_disposition() {
    for disposition in [
        Disposition::KeepOnlyEssential,
        Disposition::ScanAsContent,
        Disposition::FlagEverything,
    ] {
        let out = encoded(disposition, Anonymizer::new()).await;
        for name in ["word/document.xml", "word/styles.xml"] {
            let text = part(&out, name);
            assert!(!text.contains("private note"), "{disposition:?}: {text}");
        }
    }
}

/// Flagged content is erased with no pipeline registered at all: the built-in
/// text pipeline covers it.
#[tokio::test]
async fn a_plain_orchestrator_still_erases_what_is_flagged() {
    let registry = FormatRegistry::with_builtin();
    let policy = HiddenContentPolicy::default().with_collaboration(Disposition::FlagEverything);
    let orchestrator = Orchestrator::new().with_registry(registry.clone());
    let handle = registry
        .decode(docx(), "docx", policy)
        .await
        .expect("decode docx");
    let mut documents = [Document::new("review.docx", handle)];
    orchestrator
        .anonymize(&mut documents)
        .await
        .expect("anonymize");
    let out = documents[0].document.encode().expect("encode").to_bytes();
    let body = part(&out, "word/document.xml");
    assert!(
        !body.contains("thirty") && !body.contains("Jane Roe"),
        "{body}"
    );
    assert!(body.contains("sixty"), "{body}");
    let comments = part(&out, "word/comments.xml");
    assert!(!comments.contains("Check with legal"), "{comments}");
}
