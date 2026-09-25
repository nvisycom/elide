//! Regression test: the fold re-decodes a nested container from its *own*
//! staged redacted bytes using the container's real [`Part::hint`], not a hint
//! guessed from its id's extension.
//!
//! A container whose `Part.id` has no extension (a PDF-style object ref, an OLE
//! object) but a real `hint` used to lose its own redaction: the fold guessed an
//! empty hint from the id, `registry.decode` failed, and the container's redacted
//! body was dropped while its descendants folded on the *original* bytes, a
//! silent PII leak. This drives that exact shape through a mock container codec
//! whose nested part is keyed by an extensionless id, and asserts every level's
//! body PII is gone from the re-encoded output.
//!
//! No shipping codec produces this shape (DOCX derives its hint *from* the id
//! extension; PDF's extensionless-id parts are leaf images, not containers), so
//! the trigger only exists behind a mock format.

use elide_codec::test_util::{MOCK_EXT, MockPart, decode_mock, encode_mock, mock_format};
use elide_core::Result;
use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
use elide_core::entity::builtins::EMAIL_ADDRESS;
use elide_core::entity::{Entity, LabelCatalog};
use elide_core::modality::text::{Text, TextLocation};
use elide_core::primitive::{ComponentId, Confidence};
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, Scope, Subject};
use elide_core::test_util::MockOperator;
use elide_detection::Analyzer;
use elide_engine::{Directives, Document, Orchestrator};
use elide_format::FormatRegistry;
use elide_redaction::{Anonymizer, Rule};

/// The PII the recognizer flags and the operator replaces.
const PII: &str = "secret@example.com";
const REDACTED: &str = "[REDACTED]";

// ---- a trivial recognizer + operator that redact PII -------------------------

/// Flags every occurrence of [`PII`] in the chunk text.
struct PiiRecognizer;

#[async_trait::async_trait]
impl Recognizer<Text> for PiiRecognizer {
    fn id(&self) -> ComponentId {
        ComponentId::new("mock-pii", "1")
    }

    async fn recognize(
        &self,
        subject: &Subject<Text>,
        _ctx: &RecognizerContext<'_, Text>,
    ) -> Result<Recognition<Text>> {
        let text = subject.data().as_str();
        let mut entities = Vec::new();
        let mut from = 0;
        while let Some(rel) = text[from..].find(PII) {
            let start = from + rel;
            let end = start + PII.len();
            let loc = TextLocation::new(start, end);
            let event = AuditEvent::pattern(
                "mock-pii",
                Confidence::MAX,
                loc.clone(),
                PatternEvent::default(),
            );
            entities.push(Entity::new(
                EMAIL_ADDRESS.to_ref(),
                loc,
                AuditLog::new(event),
            ));
            from = end;
        }
        Ok(Recognition::new(entities))
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

    let handle = registry.decode(outer_bytes.clone(), MOCK_EXT).await?;
    let mut documents = [Document::new("outer.mock", handle)];

    let analyzed = orchestrator
        .analyze(&mut documents, &Directives::new())
        .await?;
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
