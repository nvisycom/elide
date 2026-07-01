//! RTF handler: a stub today.
//!
//! Decoding succeeds (so the format resolves and round-trips through the
//! registry), but no RTF content is parsed yet: streaming yields nothing,
//! reads return nothing, redaction is a no-op, and `encode` reports that
//! re-serialization is unsupported. A real implementation (an RTF
//! control-word tokenizer to pull the plain text out of the group
//! structure, and a writer to splice redactions back) will replace this
//! when RTF extraction lands.
//!
//! Unlike DOCX/PDF, RTF is *not* a container: it is a single flat stream
//! of control words and groups (`{\rtf1 … \par …}`) with the text inline,
//! so it is a leaf [`Text`] handler with no [`Container`] surface — the
//! same shape as the plain-text and markup formats.
//!
//! [`Container`]: crate::codec::Container

use elide_core::modality::text::{Text, TextData, TextLocation};
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::operator::Redactions;
use elide_core::{Error, ErrorKind, Result};

use super::RtfLoader;
use crate::content::ContentData;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the RTF codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.rtf");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), RtfLoader)
        .with_extensions(["rtf"])
        .with_content_types(["application/rtf", "text/rtf"])
}

/// Stub handler: holds nothing and exposes no text. See the module docs.
#[derive(Debug, Default)]
pub(crate) struct RtfHandler;

impl RtfHandler {
    /// An empty stub handler.
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Handler<Text> for RtfHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Err(Error::new(
            ErrorKind::Validation,
            "RTF re-encoding is not yet supported",
        ))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        Ok(None)
    }

    // No `lift` override: the stub yields no chunks, so it is never called;
    // the identity default suffices.
}

#[async_trait::async_trait]
impl DataReader<Text> for RtfHandler {
    async fn read_at(&self, _location: &TextLocation) -> Result<Option<TextData>> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl DataWriter<Text> for RtfHandler {
    async fn write_at(&mut self, _redactions: Redactions<Text>) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Loader;

    #[tokio::test]
    async fn stub_decodes_but_exposes_nothing() {
        let mut h = RtfLoader
            .decode(ContentData::new(bytes::Bytes::from_static(
                br"{\rtf1 hello}",
            )))
            .await
            .unwrap();
        assert_eq!(h.format().as_str(), "elide.document.rtf");
        // No text, no reads, redaction is a no-op, encode is unsupported.
        assert!(h.read_next().await.unwrap().is_none());
        assert!(h.read_at(&TextLocation::new(0, 0)).await.unwrap().is_none());
        assert!(h.encode().is_err());
    }
}
