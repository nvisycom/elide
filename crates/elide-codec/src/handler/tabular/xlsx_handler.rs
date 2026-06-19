//! XLSX handler: a stub today.
//!
//! Decoding succeeds (so the format resolves and round-trips through the
//! registry), but no spreadsheet content is parsed yet: streaming yields
//! nothing, reads return nothing, redaction is a no-op, and `encode`
//! reports that re-serialization is unsupported. A real parser
//! (`calamine` to read, a writer crate to re-emit) will replace this when
//! spreadsheet extraction lands.

use elide_core::modality::tabular::{Tabular, TabularLocation};
use elide_core::modality::text::TextData;
use elide_core::modality::{Chunk, DataReader, DataWriter};
use elide_core::redaction::Redactions;
use elide_core::{Error, ErrorKind, Result};

use crate::content::ContentData;
use crate::{Format, FormatId, Handler};

/// Stable [`FormatId`] for the XLSX codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.tabular.xlsx");

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Tabular, _>(FORMAT_ID.clone(), super::xlsx_loader::XlsxLoader)
        .with_extensions(["xlsx"])
        .with_content_types(["application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"])
}

/// Stub handler: holds nothing and exposes no cells. See the module docs.
#[derive(Debug, Default)]
pub(crate) struct XlsxHandler;

impl XlsxHandler {
    /// An empty stub handler.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Handler<Tabular> for XlsxHandler {
    fn format(&self) -> FormatId {
        FORMAT_ID.clone()
    }

    fn encode(&self) -> Result<ContentData> {
        Err(Error::new(
            ErrorKind::Validation,
            "XLSX re-encoding is not yet supported",
        ))
    }

    async fn read_next(&mut self) -> Result<Option<Chunk<Tabular>>> {
        Ok(None)
    }

    // No `lift` override: the stub yields no chunks, so it is never called;
    // the identity default suffices.
}

impl DataReader<Tabular> for XlsxHandler {
    async fn read_at(&self, _location: &TabularLocation) -> Result<Option<TextData>> {
        Ok(None)
    }
}

impl DataWriter<Tabular> for XlsxHandler {
    async fn write_at(&mut self, _redactions: Redactions<Tabular>) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Loader;

    #[tokio::test]
    async fn stub_decodes_but_exposes_nothing() {
        let mut h = super::super::xlsx_loader::XlsxLoader
            .decode(ContentData::new(bytes::Bytes::from_static(b"PK\x03\x04")))
            .await
            .unwrap();
        assert_eq!(h.format().as_str(), "elide.tabular.xlsx");
        // No cells, no reads, redaction is a no-op, encode is unsupported.
        assert!(h.read_next().await.unwrap().is_none());
        assert!(
            h.read_at(&TabularLocation::new(0, 0))
                .await
                .unwrap()
                .is_none()
        );
        assert!(h.encode().is_err());
    }
}
