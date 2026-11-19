//! [`XlsxDocumentLoader`]: decode a workbook into a [`Document`].

use elide_codec::content::ContentData;
use elide_codec::extract::ExtractStream;
use elide_codec::{Document, DocumentLoader};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::FORMAT_ID;
use super::document::XlsxDocument;
use super::state::XlsxState;
use super::stream::XlsxStream;
use crate::codec::ooxml::{OoxmlAddresser, blobs, items, refuse_issues, sanitized};
use crate::codec::props::DocProps;
use crate::xlsx::{SheetClassifier, Xlsx};

/// A [`DocumentLoader`] that decodes an XLSX workbook into a [`Document`]: an
/// [`XlsxDocument`] (the cell stream, a text stream over the workbook's content
/// outside its cells, and, when metadata is treated as content, a metadata
/// stream per document-property part) plus a [`Blob`](elide_codec::Blob) per embedding. The
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
        let (archive, stripped) = sanitized(&content, policy, &SheetClassifier)?;
        let workbook = Xlsx::open(&archive)?;
        let state = XlsxState::new(workbook.extract()?);

        let markup = workbook.markup();
        refuse_issues("xlsx", &markup.issues)?;

        // One blob per embedding, each decoded through its own format's pipeline.
        let blobs = blobs(markup.embeddings);
        let properties = DocProps::under(policy, |path| workbook.part_bytes(path));
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
