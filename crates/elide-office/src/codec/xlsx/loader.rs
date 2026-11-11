//! [`XlsxDocumentLoader`]: decode a workbook into a parts-model [`Document`].

use std::sync::Arc;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::{ExtractStream, SharedSplice};
use elide_codec::{Document, DocumentLoader, DocumentPart, ErasedStream, LocalId, Stream};
use elide_core::modality::tabular::Tabular;
use elide_core::modality::text::Text;
use elide_core::redaction::{Disposition, HiddenContentPolicy};
use elide_core::{Error, Result};

use super::recombine::XlsxRecombine;
use super::state::XlsxState;
use super::stream::XlsxStream;
use super::{BODY_PART_ID, FORMAT_ID, TEXT_PART_ID, XlsxCell};
use crate::codec::ooxml::{OoxmlAddress, OoxmlAddresser, items};
use crate::codec::props::doc_props_parts;
use crate::opc::sanitize::sanitize;
use crate::xlsx::{SheetClassifier, Xlsx};

/// A [`DocumentLoader`] that decodes an XLSX workbook into a [`Document`]: the
/// cell body [`Stream`], a text [`Stream`] over the workbook's content outside
/// its cells (comments, drawings, charts, external hyperlink targets), a
/// [`Blob`](DocumentPart::Blob) per embedding, and (when metadata is treated as
/// content) a metadata stream per document-property part. The workbook is sanitized under the policy
/// first, so every part is read from what will ship.
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
        let text: SharedSplice<OoxmlAddress> = SharedSplice::new(items(markup.blocks));

        // The cell body, the non-cell text, then one blob per embedding (its own
        // format's pipeline) and a metadata stream per document-property part.
        let cells_stream: Box<dyn Stream<Tabular>> = Box::new(XlsxStream {
            state: state.clone(),
        });
        let text_stream: Box<dyn Stream<Text>> = Box::new(ExtractStream::new(
            FORMAT_ID,
            text.clone(),
            Arc::new(OoxmlAddresser),
        ));
        let streams = [
            DocumentPart::Stream {
                id: LocalId::new(BODY_PART_ID),
                handle: ErasedStream::new(cells_stream),
            },
            DocumentPart::Stream {
                id: LocalId::new(TEXT_PART_ID),
                handle: ErasedStream::new(text_stream),
            },
        ];
        let embeddings = markup.embeddings.into_iter().map(|embedding| {
            let id = LocalId::new(embedding.part.as_str().to_owned());
            let hint = id.extension().unwrap_or_default().to_owned();
            DocumentPart::Blob {
                id,
                bytes: embedding.bytes,
                hint,
            }
        });
        let properties = match policy.metadata {
            Disposition::ScanAsContent | Disposition::FlagEverything => {
                doc_props_parts(|path| workbook.part_bytes(path))
            }
            Disposition::KeepOnlyEssential => Vec::new(),
        };
        let parts = streams
            .into_iter()
            .chain(embeddings)
            .chain(properties)
            .collect();

        let recombine = XlsxRecombine {
            archive,
            state,
            text,
            policy,
        };
        Ok(Document::new(FORMAT_ID, parts, Box::new(recombine)).with_stripped(stripped))
    }
}
