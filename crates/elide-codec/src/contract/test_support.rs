//! Throwaway unit-test doubles shared across the `contract` unit tests: a
//! minimal one-string [`Stream<Text>`] and a native document that only joins its
//! blobs. These are crate-internal, not the cross-crate mock surface (see
//! `crate::mocks`).

use elide_core::Result;
use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;

use super::{Blob, Encode, FormatId, LocalId, Native, Stream, StreamMut};
use crate::content::ContentData;

/// A minimal text stream holding one editable string, which it encodes as the
/// document. It yields no chunks and reads nothing (the tests exercise wiring,
/// not chunking).
pub(crate) struct StrStream(pub String);

impl Stream<Text> for StrStream {
    fn format(&self) -> FormatId {
        FormatId::new("elide.test.str")
    }

    fn chunks(&self) -> Result<Vec<Chunk<Text>>> {
        Ok(Vec::new())
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

impl Encode for StrStream {
    fn encode(&self) -> Result<ContentData> {
        Ok(ContentData::from_text(self.0.clone()))
    }
}

/// A native document with no streams that concatenates its blobs' bytes, so a
/// test can assert the blob order and the fold.
pub(crate) struct JoinBlobs;

impl Native for JoinBlobs {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        Vec::new()
    }

    fn encode(&self, blobs: &[Blob], _removed: &[LocalId]) -> Result<ContentData> {
        let mut out = Vec::new();
        for blob in blobs {
            out.extend_from_slice(&blob.bytes);
        }
        Ok(ContentData::from(out))
    }

    fn validate(&self, _output: &ContentData) -> Result<()> {
        Ok(())
    }
}
