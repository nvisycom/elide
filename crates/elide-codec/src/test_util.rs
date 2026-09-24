//! A mock codec format, behind the `test-util` feature.
//!
//! For exercising registry, handler, and orchestration behavior without a real
//! file format.
//!
//! [`MockHandler`] is a functional [`Text`] handler: it decodes a body plus any
//! number of embedded [`Part`]s, streams the body as one chunk, redacts it by
//! byte range, and folds staged part replacements back on encode. Each part
//! carries a decoder [`hint`](Part) independent of its id, so a test can key a
//! part by an extensionless id yet still name a real hint — the shape a
//! container fold must honor.
//!
//! # Wire format
//!
//! One `\n`-separated line each:
//! - line 0: the body text (never contains `@PART`).
//! - line 1+: `@PART <id> <hint> <hex>`, one embedded part, `<hex>` its bytes.
//!
//! [`Text`]: elide_core::modality::text::Text

use bytes::Bytes;
use elide_core::Result;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use crate::content::ContentData;
use crate::string::RedactRange;
use crate::{Container, Format, FormatId, Handler, Loader, LocalId, Part};

/// Stable [`FormatId`] for the mock format.
pub const MOCK_FORMAT_ID: FormatId = FormatId::new("elide.test.mock");

/// The extension the registry resolves the mock format on.
pub const MOCK_EXT: &str = "mock";

/// One embedded part of a [`MockHandler`]: its local id, its decoder hint, and
/// its raw bytes. The hint is stored independent of the id so a part can be
/// keyed by an extensionless id yet still carry a real hint.
#[derive(Clone, Debug)]
pub struct MockPart {
    /// The part's container-local id.
    pub id: String,
    /// The decoder hint the fold resolves the part by.
    pub hint: String,
    /// The part's raw bytes.
    pub bytes: Bytes,
}

/// Serialize a body plus embedded parts into the mock wire format.
#[must_use]
pub fn encode_mock(body: &str, parts: &[MockPart]) -> Bytes {
    let mut out = body.to_owned();
    for p in parts {
        out.push_str(&format!("\n@PART {} {} {}", p.id, p.hint, hex(&p.bytes)));
    }
    Bytes::from(out.into_bytes())
}

/// Parse the mock wire format back into a body and its embedded parts.
#[must_use]
pub fn decode_mock(bytes: &[u8]) -> (String, Vec<MockPart>) {
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
            // round trip is faithful (callers never produce one).
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

/// A mock [`Text`] handler: a text body plus embedded parts, with staged
/// part replacements.
#[derive(Debug)]
pub struct MockHandler {
    body: String,
    parts: Vec<MockPart>,
    /// Redacted bytes staged through [`Container::replace_part`], keyed by id.
    replaced: std::collections::HashMap<String, Bytes>,
    /// Streaming cursor: the body is a single chunk.
    yielded: bool,
}

impl MockHandler {
    /// Decode a mock document from its wire bytes.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Self {
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
            let value = replacement.value().unwrap_or_default();
            // `redact_range` clamps the endpoints and errors on a mid-character
            // boundary rather than panicking like `String::replace_range`.
            self.body.redact_range(value, range.start..range.end)?;
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

/// A loader that decodes the mock wire format into a [`MockHandler`].
#[derive(Debug)]
pub struct MockLoader;

#[async_trait::async_trait]
impl Loader for MockLoader {
    type Handler = MockHandler;
    type Modality = Text;

    async fn decode(&self, content: ContentData) -> Result<MockHandler> {
        Ok(MockHandler::parse(content.as_bytes()))
    }
}

/// The mock [`Format`], registered on [`MOCK_EXT`].
#[must_use]
pub fn mock_format() -> Format {
    Format::new(MOCK_FORMAT_ID.clone(), MockLoader).with_extensions([MOCK_EXT])
}
