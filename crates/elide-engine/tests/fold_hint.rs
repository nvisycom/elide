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

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{Container, Format, FormatId, FormatRegistry, Handler, Loader, LocalId, Part};
use elide_core::Result;
use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
use elide_core::entity::builtins::EMAIL_ADDRESS;
use elide_core::entity::{Entity, LabelCatalog};
use elide_core::modality::text::{Text, TextData, TextLocation, TextReplacement};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::primitive::Confidence;
use elide_core::recognition::{Recognition, Recognizer, RecognizerContext, RecognizerId, Scope};
use elide_core::redaction::{LeakProfile, Operator, OperatorId, Redactions};
use elide_detection::Analyzer;
use elide_engine::{Directives, Document, Orchestrator};
use elide_redaction::{Anonymizer, Rule};

const MOCK_FORMAT_ID: FormatId = FormatId::new("elide.test.mock");
/// The extension the registry resolves the top-level mock document on.
const MOCK_EXT: &str = "mock";
/// The PII the recognizer flags and the operator replaces.
const PII: &str = "secret@example.com";
const REDACTED: &str = "[REDACTED]";

// ---- the mock container format -----------------------------------------------
//
// Byte format, one `\n`-separated line each:
//   line 0:  the body text (never contains `@PART`).
//   line 1+: `@PART <id> <hint> <hex>` , one embedded part, `<hex>` its bytes.
//
// A part's `hint` is stored independently of its id, so a part can be keyed by
// an extensionless id (`inner`) yet still carry a real decoder hint (`mock`),
// the exact mismatch the fold must honor.

/// One embedded part: its local id, its decoder hint, and its raw bytes.
#[derive(Clone)]
struct MockPart {
    id: String,
    hint: String,
    bytes: Bytes,
}

/// Serialize a body plus embedded parts into the mock wire format.
fn encode_mock(body: &str, parts: &[MockPart]) -> Bytes {
    let mut out = body.to_owned();
    for p in parts {
        out.push_str(&format!("\n@PART {} {} {}", p.id, p.hint, hex(&p.bytes)));
    }
    Bytes::from(out.into_bytes())
}

/// Parse the mock wire format back into a body and its embedded parts.
fn decode_mock(bytes: &[u8]) -> (String, Vec<MockPart>) {
    let text = String::from_utf8_lossy(bytes);
    let mut body = String::new();
    let mut parts = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        if let Some(rest) = line.strip_prefix("@PART ") {
            let mut it = rest.splitn(3, ' ');
            let id = it.next().unwrap_or_default().to_owned();
            let hint = it.next().unwrap_or_default().to_owned();
            let bytes = it.next().map(unhex).unwrap_or_default();
            parts.push(MockPart { id, hint, bytes });
        } else if i == 0 {
            body = line.to_owned();
        } else {
            // A stray non-part line after the body: keep it on the body so a
            // round trip is faithful (the tests never produce one).
            body.push('\n');
            body.push_str(line);
        }
    }
    (body, parts)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn unhex(s: &str) -> Bytes {
    let bytes: Vec<u8> = (0..s.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect();
    Bytes::from(bytes)
}

/// The mock handler: a text body plus embedded parts, with staged replacements.
struct MockHandler {
    body: String,
    parts: Vec<MockPart>,
    /// Redacted bytes staged through `replace_part`, keyed by part id.
    replaced: std::collections::HashMap<String, Bytes>,
    /// Streaming cursor: the body is a single chunk.
    yielded: bool,
}

impl MockHandler {
    fn parse(bytes: &[u8]) -> Self {
        let (body, parts) = decode_mock(bytes);
        Self {
            body,
            parts,
            replaced: std::collections::HashMap::new(),
            yielded: false,
        }
    }
}

#[async_trait::async_trait]
impl Handler<Text> for MockHandler {
    fn format(&self) -> FormatId {
        MOCK_FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        // Re-serialize, substituting any staged replacement bytes for a part.
        let parts: Vec<MockPart> = self
            .parts
            .iter()
            .map(|p| MockPart {
                id: p.id.clone(),
                hint: p.hint.clone(),
                bytes: self.replaced.get(&p.id).cloned().unwrap_or(p.bytes.clone()),
            })
            .collect();
        Ok(ContentData::new(encode_mock(&self.body, &parts)))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        if self.yielded {
            return Ok(None);
        }
        self.yielded = true;
        Ok(Some(Chunk {
            location: TextLocation::new(0, self.body.len()),
            data: TextData::new(self.body.clone()),
            hints: Vec::new(),
        }))
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        Some(self)
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for MockHandler {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(range) = location.range() else {
            return Ok(None);
        };
        Ok(self.body.get(range.start..range.end).map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for MockHandler {
    async fn write_at(&mut self, mut redactions: Redactions<Text>) -> Result<()> {
        // Apply right-to-left so each edit's length delta leaves earlier
        // locations valid.
        redactions.sort_by_position();
        for (location, replacement) in redactions.into_iter().rev() {
            let Some(range) = location.range() else {
                continue;
            };
            let range = range.start..range.end;
            if range.end <= self.body.len() {
                let value = replacement.value().unwrap_or_default();
                self.body.replace_range(range, value);
            }
        }
        Ok(())
    }
}

impl Container for MockHandler {
    fn parts(&self) -> Vec<Part> {
        self.parts
            .iter()
            .map(|p| Part {
                id: LocalId::new(p.id.clone()),
                bytes: p.bytes.clone(),
                hint: p.hint.clone(),
            })
            .collect()
    }

    fn replace_part(&mut self, id: &LocalId, bytes: Bytes) -> Result<()> {
        self.replaced.insert(id.as_str().to_owned(), bytes);
        Ok(())
    }
}

struct MockLoader;

#[async_trait::async_trait]
impl Loader<Text> for MockLoader {
    type Handler = MockHandler;

    async fn decode(&self, content: ContentData) -> Result<MockHandler> {
        Ok(MockHandler::parse(content.as_bytes()))
    }
}

fn mock_format() -> Format {
    Format::new::<Text, _>(MOCK_FORMAT_ID.clone(), MockLoader).with_extensions([MOCK_EXT])
}

// ---- a trivial recognizer + operator that redact PII -------------------------

/// Flags every occurrence of [`PII`] in the chunk text.
struct PiiRecognizer;

#[async_trait::async_trait]
impl Recognizer<Text> for PiiRecognizer {
    fn id(&self) -> RecognizerId {
        RecognizerId::new("mock-pii", "1")
    }

    async fn recognize(
        &self,
        data: &TextData,
        _ctx: &RecognizerContext<'_, Text>,
    ) -> Result<Recognition<Text>> {
        let text = data.as_str();
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

/// Replaces a matched span with [`REDACTED`].
struct RedactOp;

#[async_trait::async_trait]
impl Operator<Text> for RedactOp {
    fn id(&self) -> OperatorId {
        OperatorId::new("mock-redact", "1")
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Irrecoverable
    }

    async fn anonymize(&self, _entity: &Entity<Text>, _data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(REDACTED))
    }
}

fn orchestrator(registry: FormatRegistry) -> Orchestrator {
    let analyzer = Analyzer::new().with_recognizer(PiiRecognizer);
    let anonymizer = Anonymizer::new().with(Rule::fallback(RedactOp));
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
    let out = documents[0].handle.encode()?.to_bytes();
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
