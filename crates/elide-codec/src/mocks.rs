//! A mock codec format for tests, behind the `mocks` feature.
//!
//! For exercising registry, stream, and orchestration behavior without a real
//! file format.
//!
//! The mock document is a [`Document`] of a body [`Stream`] plus any number of
//! [`Blob`]s: [`MockStream`] streams the body as one chunk and redacts it by
//! byte range, and [`MockDocument`] re-serializes the body with the (possibly
//! redacted) blob bytes. Each blob carries a decoder [`hint`](Blob::hint)
//! independent of its id, so a test can key a part by an extensionless id yet
//! still name a real hint — the shape a container fold must honor.
//!
//! # Wire format
//!
//! One `\n`-separated line each:
//! - line 0: the body text (never contains `@PART`).
//! - line 1+: `@PART <id> <hint> <hex>`, one embedded part, `<hex>` its bytes.
//!
//! [`Text`]: elide_core::modality::text::Text

use bytes::Bytes;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::{HiddenContentPolicy, Redactions};
use elide_core::{Error, Result};

use crate::content::ContentData;
use crate::string::RedactRange;
use crate::{Blob, Document, DocumentLoader, Format, FormatId, LocalId, Native, Stream, StreamMut};

/// The stable [`FormatId`] of the mock format, `elide.test.mock`.
pub const MOCK_FORMAT_ID: FormatId = FormatId::new("elide.test.mock");

/// The extension the registry resolves the mock format on.
pub const MOCK_EXT: &str = "mock";

/// One embedded file of a mock document: its id, decoder hint, and bytes.
///
/// The hint is stored independent of the id, so a file can be keyed by an
/// extensionless id yet still carry a real hint.
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
        out.push_str(&format!(
            "\n@PART {} {} {}",
            p.id,
            p.hint,
            hex::encode(&p.bytes)
        ));
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
            let bytes = it
                .next()
                .and_then(|hex| hex::decode(hex).ok())
                .map(Bytes::from)
                .unwrap_or_default();
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

/// The id of a mock document's body, the one stream it has.
pub const MOCK_BODY_ID: &str = "body";

/// The body [`Stream`] of a mock document: one editable line of text.
///
/// It is yielded as one chunk and redacted by byte range; [`MockDocument`]
/// writes it out with the blobs.
#[derive(Debug)]
pub struct MockStream {
    body: String,
}

impl Stream<Text> for MockStream {
    fn format(&self) -> FormatId {
        MOCK_FORMAT_ID
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        Ok(vec![Chunk {
            location: TextLocation::inline(0..self.body.len()),
            data: TextData::new(self.body.clone()),
            hints: Vec::new(),
        }])
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for MockStream {
    async fn read_at(&self, location: &TextLocation) -> Result<TextData> {
        location
            .range()
            .and_then(|range| self.body.get(range.clone()))
            .map(TextData::new)
            .ok_or_else(|| Error::malformed("the location addresses no mock text"))
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

/// The native mock document: the body [`MockStream`] and its blob lines.
///
/// It re-serializes with each blob's `@PART <id> <hint> <hex>` line. A blob's
/// bytes are its current (possibly redacted) ones; a removed blob is not
/// written.
#[derive(Debug)]
pub struct MockDocument {
    body: MockStream,
}

impl Native for MockDocument {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        vec![(
            LocalId::new(MOCK_BODY_ID),
            StreamMut::new::<Text, _>(&mut self.body),
        )]
    }

    fn encode(&self, blobs: &[Blob], _removed: &[LocalId]) -> Result<ContentData> {
        let parts: Vec<MockPart> = blobs
            .iter()
            .map(|blob| MockPart {
                id: blob.id.as_str().to_owned(),
                hint: blob.hint.clone(),
                bytes: blob.bytes.clone(),
            })
            .collect();
        Ok(ContentData::new(encode_mock(&self.body.body, &parts)))
    }

    /// The mock wire format carries no hidden content, so there is nothing to
    /// check.
    fn validate(&self, _output: &ContentData) -> Result<()> {
        Ok(())
    }
}

/// A [`DocumentLoader`] that decodes the mock wire format into a [`Document`].
///
/// The document is a [`MockDocument`] plus one [`Blob`] per embedded part.
#[derive(Debug)]
pub struct MockLoader;

#[async_trait::async_trait]
impl DocumentLoader for MockLoader {
    async fn decode(&self, content: ContentData, _policy: HiddenContentPolicy) -> Result<Document> {
        let (body, parts) = decode_mock(content.as_bytes());
        let blobs = parts
            .into_iter()
            .map(|part| Blob {
                id: LocalId::new(part.id),
                bytes: part.bytes,
                hint: part.hint,
            })
            .collect();
        let native = MockDocument {
            body: MockStream { body },
        };
        Ok(Document::new(MOCK_FORMAT_ID, native, blobs))
    }
}

/// The mock [`Format`], resolved by its [`MOCK_EXT`] extension, for tests.
#[must_use]
pub fn mock_format() -> Format {
    Format::with_document_loader(MOCK_FORMAT_ID, MockLoader).with_extensions([MOCK_EXT])
}
