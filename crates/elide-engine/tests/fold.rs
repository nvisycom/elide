//! How a nested container folds back into its parent on redaction.
//!
//! - A nested container is re-decoded by its blob's own hint, not one guessed
//!   from its id, so a container under an extensionless id (a PDF object ref,
//!   an OLE object) keeps its own redaction.
//! - An embedded document nothing was written to keeps its original bytes.
//!
//! Both run through the mock container codec: no shipping codec nests a
//! container under an extensionless id.

use elide_codec::mocks::{MOCK_EXT, MockPart, decode_mock, encode_mock, mock_format};
use elide_core::entity::audit::{AuditEvent, PatternRule};
use elide_core::entity::builtins::EMAIL_ADDRESS;
use elide_core::entity::{Entity, LabelCatalog};
use elide_core::mocks::MockOperator;
use elide_core::modality::text::{Text, TextLocation};
use elide_core::primitive::Confidence;
use elide_core::recognition::{Context, Recognizer, Scope, Subject};
use elide_core::redaction::HiddenContentPolicy;
use elide_core::{Component, Result};
use elide_detection::Analyzer;
use elide_engine::{Document, Orchestrator};
use elide_format::FormatRegistry;
use elide_redaction::{Anonymizer, Rule};

/// The PII the recognizer flags and the operator replaces.
const PII: &str = "secret@example.com";
const REDACTED: &str = "[REDACTED]";

// ---- a trivial recognizer + operator that redact PII -------------------------

/// Flags every occurrence of [`PII`] in the chunk text.
#[derive(Component)]
#[component(name = "mock-pii", version = "1")]
struct PiiRecognizer;

#[async_trait::async_trait]
impl Recognizer<Text> for PiiRecognizer {
    async fn recognize(
        &self,
        subject: &Subject<Text>,
        _ctx: &Context<'_>,
    ) -> Result<Vec<Entity<Text>>> {
        let text = subject.data().as_str();
        let mut entities = Vec::new();
        let mut from = 0;
        while let Some(rel) = text[from..].find(PII) {
            let start = from + rel;
            let end = start + PII.len();
            let loc = TextLocation::inline(start..end);
            let event = AuditEvent::pattern(
                "mock-pii",
                Confidence::MAX,
                loc.clone(),
                PatternRule::default(),
            );
            entities.push(Entity::new(EMAIL_ADDRESS.to_ref(), loc).with_event(event));
            from = end;
        }
        Ok(entities)
    }
}

fn orchestrator(registry: FormatRegistry) -> Orchestrator {
    let analyzer = Analyzer::new().with_recognizer(PiiRecognizer);
    let anonymizer = Anonymizer::new().with(Rule::fallback(MockOperator::new(REDACTED)));
    Orchestrator::new()
        .with_scope(Scope::new().with_catalog(LabelCatalog::with_builtins()))
        .with_registry(registry)
        .with_modality::<Text>(analyzer, anonymizer)
}

/// The container's body carries a distinct PII, tagged by level, so a leak at
/// any level is identifiable.
fn body_for(level: &str) -> String {
    format!("{level} body holds {PII} here")
}

#[tokio::test]
async fn nested_container_keeps_its_own_redaction_with_an_extensionless_id() -> Result<()> {
    // leaf (a plain text part, id `leaf.txt`) embedded in inner (a container at
    // the EXTENSIONLESS id `inner`, hint `mock`) embedded in outer.
    let leaf_bytes = encode_mock(&body_for("leaf"), &[]);
    let inner_bytes = encode_mock(
        &body_for("inner"),
        &[MockPart {
            id: "leaf.txt".to_owned(),
            hint: MOCK_EXT.to_owned(),
            bytes: leaf_bytes,
        }],
    );
    let outer_bytes = encode_mock(
        &body_for("outer"),
        &[MockPart {
            // No extension on the id: the fold cannot guess the hint from it.
            id: "inner".to_owned(),
            hint: MOCK_EXT.to_owned(),
            bytes: inner_bytes,
        }],
    );

    let registry = FormatRegistry::new().with_format(mock_format());
    let orchestrator = orchestrator(registry.clone());

    let handle = registry
        .decode(
            outer_bytes.clone(),
            MOCK_EXT,
            HiddenContentPolicy::default(),
        )
        .await?;
    let mut documents = [Document::new("outer.mock", handle)];

    let analyzed = orchestrator.analyze(&mut documents).await?;
    // Every level's body is reached (outer depth 1, inner depth 2, leaf depth 3).
    let depths: Vec<usize> = analyzed
        .report
        .part_ids()
        .map(|(id, _)| id.depth())
        .collect();
    assert!(
        depths.contains(&1) && depths.contains(&2) && depths.contains(&3),
        "every nesting level's body is analyzed; got depths {depths:?}",
    );

    orchestrator
        .anonymize_with(&mut documents, analyzed.report)
        .await?;

    // Re-encode and walk the tree: no level's body PII may survive. The inner
    // assertion is the regression: under the old id-extension hint, `inner`'s
    // empty extension made the re-decode fail and dropped its own redaction.
    let out = documents[0].document.encode()?.to_bytes();
    let (outer_body, outer_parts) = decode_mock(&out);
    assert!(!outer_body.contains(PII), "outer body leaked: {outer_body}");

    let inner = outer_parts
        .iter()
        .find(|p| p.id == "inner")
        .expect("inner part present");
    let (inner_body, inner_parts) = decode_mock(&inner.bytes);
    assert!(
        !inner_body.contains(PII),
        "the nested container's OWN body redaction was lost: {inner_body}",
    );

    let leaf = inner_parts
        .iter()
        .find(|p| p.id == "leaf.txt")
        .expect("leaf part present");
    let (leaf_body, _) = decode_mock(&leaf.bytes);
    assert!(!leaf_body.contains(PII), "leaf body leaked: {leaf_body}");

    Ok(())
}

#[tokio::test]
async fn an_embedded_document_nothing_was_redacted_in_keeps_its_bytes() -> Result<()> {
    // Not valid UTF-8, so a decode and re-encode would replace the byte and
    // show up as changed bytes.
    let mut clean = b"clean body ".to_vec();
    clean.push(0xFF);
    let outer_bytes = encode_mock(
        &body_for("outer"),
        &[MockPart {
            id: "clean.mock".to_owned(),
            hint: MOCK_EXT.to_owned(),
            bytes: clean.clone().into(),
        }],
    );

    let registry = FormatRegistry::new().with_format(mock_format());
    let orchestrator = orchestrator(registry.clone());
    let handle = registry
        .decode(outer_bytes, MOCK_EXT, HiddenContentPolicy::default())
        .await?;
    let mut documents = [Document::new("outer.mock", handle)];
    orchestrator.anonymize(&mut documents).await?;

    let (outer_body, outer_parts) = decode_mock(&documents[0].document.encode()?.to_bytes());
    assert!(!outer_body.contains(PII), "outer body leaked: {outer_body}");
    let embedded = outer_parts
        .iter()
        .find(|p| p.id == "clean.mock")
        .expect("embedded part present");
    assert_eq!(embedded.bytes.as_ref(), clean.as_slice());
    Ok(())
}
