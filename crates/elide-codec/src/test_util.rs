//! A mock codec format, behind the `test-util` feature.
//!
//! For exercising registry, handler, and orchestration behavior without a real
//! file format.
//!
//! The mock document is a [`Document`] of a body [`Stream`] plus any number of
//! [`Blob`](DocumentPart::Blob) sub-parts: [`MockStream`] streams the body as one
//! chunk and redacts it by byte range, and [`MockRecombine`] re-serializes the
//! body with the (possibly redacted) blob bytes. Each blob carries a decoder
//! [`hint`](DocumentPart::Blob) independent of its id, so a test can key a part
//! by an extensionless id yet still name a real hint — the shape a container fold
//! must honor.
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
use crate::{
    Document, DocumentLoader, DocumentPart, EncodedPart, ErasedStream, Format, FormatId, LocalId,
    Recombine, Stream,
};

/// Stable [`FormatId`] for the mock format.
pub const MOCK_FORMAT_ID: FormatId = FormatId::new("elide.test.mock");

/// The extension the registry resolves the mock format on.
pub const MOCK_EXT: &str = "mock";

/// One embedded blob part of a mock document: its local id, its decoder hint,
/// and its raw bytes. The hint is stored independent of the id so a part can be
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

/// The stream part id of a mock document's body.
pub const MOCK_BODY_ID: &str = "body";

/// The body [`Stream`] of a mock document: an editable text line, streamed as one
/// chunk and redacted by byte range. Its [`encode`](Stream::encode) yields the
/// body alone; [`MockRecombine`] re-attaches the blob parts.
#[derive(Debug)]
pub struct MockStream {
    body: String,
    yielded: bool,
}

#[async_trait::async_trait]
impl Stream<Text> for MockStream {
    fn format(&self) -> FormatId {
        MOCK_FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        // The body only; the recombiner re-attaches the blob parts.
        Ok(ContentData::from_text(self.body.clone()))
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
}

#[async_trait::async_trait]
impl DataReader<Text> for MockStream {
    async fn read_at(&self, location: &TextLocation) -> Result<Option<TextData>> {
        let Some(range) = location.range() else {
            return Ok(None);
        };
        Ok(self.body.get(range.start..range.end).map(TextData::new))
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for MockStream {
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

/// The recombiner for a mock document: re-serialize the body (the [`MockStream`]'s
/// re-encoded bytes) with each blob part's `@PART <id> <hint> <hex>` line, using
/// the decode-time hints. A blob's bytes are its current (possibly redacted) ones.
#[derive(Debug)]
pub struct MockRecombine {
    /// The `(id, hint)` of each blob part, in document order, captured at decode.
    hints: Vec<(String, String)>,
}

impl Recombine for MockRecombine {
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData> {
        let body = parts
            .iter()
            .find(|p| p.id.as_str() == MOCK_BODY_ID)
            .map(|p| String::from_utf8_lossy(&p.bytes).into_owned())
            .unwrap_or_default();
        let blobs: Vec<MockPart> = self
            .hints
            .iter()
            .filter_map(|(id, hint)| {
                let bytes = parts.iter().find(|p| p.id.as_str() == id)?.bytes.clone();
                Some(MockPart {
                    id: id.clone(),
                    hint: hint.clone(),
                    bytes,
                })
            })
            .collect();
        Ok(ContentData::new(encode_mock(&body, &blobs)))
    }
}

/// A [`DocumentLoader`] that decodes the mock wire format into a [`Document`]: a
/// body [`MockStream`] plus one [`Blob`](DocumentPart::Blob) per embedded part.
#[derive(Debug)]
pub struct MockLoader;

#[async_trait::async_trait]
impl DocumentLoader for MockLoader {
    async fn decode(&self, content: ContentData) -> Result<Document> {
        let (body, parts) = decode_mock(content.as_bytes());
        let hints = parts
            .iter()
            .map(|p| (p.id.clone(), p.hint.clone()))
            .collect();

        let mut document_parts = Vec::with_capacity(parts.len() + 1);
        document_parts.push(DocumentPart::Stream {
            id: LocalId::new(MOCK_BODY_ID),
            handle: ErasedStream::new(
                MOCK_FORMAT_ID.clone(),
                Box::new(MockStream {
                    body,
                    yielded: false,
                }) as Box<dyn Stream<Text>>,
            ),
        });
        for p in parts {
            document_parts.push(DocumentPart::Blob {
                id: LocalId::new(p.id),
                bytes: p.bytes,
                hint: p.hint,
            });
        }

        Ok(Document::new(
            MOCK_FORMAT_ID.clone(),
            document_parts,
            Box::new(MockRecombine { hints }),
        ))
    }
}

/// The mock [`Format`], registered on [`MOCK_EXT`].
#[must_use]
pub fn mock_format() -> Format {
    Format::with_document_loader(MOCK_FORMAT_ID.clone(), MockLoader).with_extensions([MOCK_EXT])
}
