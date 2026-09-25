//! Throwaway unit-test doubles shared across the `contract` unit tests: a
//! minimal one-string [`Stream<Text>`] and a concatenating [`Recombine`]. These
//! are crate-internal, not the cross-crate mock surface (see `crate::test_util`).

use elide_core::Result;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::{EncodedPart, FormatId, Recombine, Stream};
use crate::content::ContentData;

/// A minimal text stream holding one editable string. Its `encode` returns the
/// string; it yields no chunks and reads nothing (the tests exercise wiring, not
/// chunking).
pub(crate) struct StrStream(pub String);

#[async_trait::async_trait]
impl Stream<Text> for StrStream {
    fn format(&self) -> FormatId {
        FormatId::new("elide.test.str")
    }

    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::from_text(self.0.clone()))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl DataReader<Text> for StrStream {
    async fn read_at(&self, _location: &TextLocation) -> Result<Option<TextData>> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for StrStream {
    async fn write_at(&mut self, _redactions: Redactions<Text>) -> Result<()> {
        Ok(())
    }
}

/// A recombiner that concatenates every part's bytes, so a test can assert the
/// assembled order and the blob-fold.
pub(crate) struct JoinRecombine;

impl Recombine for JoinRecombine {
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData> {
        let mut out = Vec::new();
        for part in parts {
            out.extend_from_slice(&part.bytes);
        }
        Ok(ContentData::new(bytes::Bytes::from(out)))
    }
}
