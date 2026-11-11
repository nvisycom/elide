//! [`XlsxDocumentLoader`]: decode a workbook into a [`Document`].

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::ExtractStream;
use elide_codec::{Blob, Document, DocumentLoader, LocalId};
use elide_core::redaction::{Disposition, HiddenContentPolicy};
use elide_core::{Error, Result};

use super::document::XlsxDocument;
use super::state::XlsxState;
use super::stream::XlsxStream;
use super::{FORMAT_ID, XlsxCell};
use crate::codec::ooxml::{OoxmlAddresser, items};
use crate::codec::props::DocProps;
use crate::opc::sanitize::sanitize;
use crate::xlsx::{SheetClassifier, Xlsx};

/// A [`DocumentLoader`] that decodes an XLSX workbook into a [`Document`]: an
/// [`XlsxDocument`] (the cell stream, a text stream over the workbook's content
/// outside its cells, and, when metadata is treated as content, a metadata
/// stream per document-property part) plus a [`Blob`] per embedding. The
/// workbook is sanitized under the policy first, so every part is read from what
/// will ship.
///
/// **Fail-closed:** a non-cell text part that cannot be read refuses the
/// decode, since its text would ship un-redacted.
#[derive(Debug)]
pub(crate) struct XlsxDocumentLoader;

#[async_trait::async_trait]
impl DocumentLoader for XlsxDocumentLoader {
    async fn decode(&self, content: ContentData, policy: HiddenContentPolicy) -> Result<Document> {
        let source = content.to_bytes();
        let (sanitized, stripped) = sanitize(&source, policy, &SheetClassifier)?;
        let archive = sanitized.map_or(source, Bytes::from);
        let workbook = Xlsx::open(&archive)?;
        let cells: Vec<XlsxCell> = workbook
            .extract()?
            .into_iter()
            .map(|cell| XlsxCell {
                sheet: cell.sheet.as_str().to_owned(),
                row: cell.row,
                column: cell.column,
                text: cell.text.as_str().to_owned(),
            })
            .collect();
        let state = XlsxState::new(cells);

        let markup = workbook.markup();
        if let Some(issue) = markup.issues.first() {
            return Err(Error::malformed(format!(
                "xlsx: text-bearing part `{}` could not be extracted ({:?})",
                issue.part, issue.kind
            )));
        }

        // One blob per embedding, each decoded through its own format's pipeline.
        let blobs = markup
            .embeddings
            .into_iter()
            .map(|embedding| {
                let id = LocalId::new(embedding.part.as_str().to_owned());
                let hint = id.extension().unwrap_or_default().to_owned();
                Blob {
                    id,
                    bytes: embedding.bytes,
                    hint,
                }
            })
            .collect();
        let properties = match policy.metadata {
            Disposition::ScanAsContent | Disposition::FlagEverything => {
                DocProps::read(|path| workbook.part_bytes(path))
            }
            Disposition::KeepOnlyEssential => DocProps::default(),
        };
        let native = XlsxDocument {
            cells: XlsxStream { state },
            text: ExtractStream::new(FORMAT_ID, items(markup.blocks), OoxmlAddresser),
            properties,
            archive,
            policy,
        };
        Ok(Document::new(FORMAT_ID, native, blobs).with_stripped(stripped))
    }
}
