//! [`XlsxDocument`]: a decoded workbook, re-packed from its redacted streams and
//! blobs, then checked.

use std::iter;

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::ExtractStream;
use elide_codec::{Blob, LocalId, Native, StreamMut};
use elide_core::Result;
use elide_core::modality::tabular::Tabular;
use elide_core::modality::text::Text;
use elide_core::redaction::HiddenContentPolicy;

use super::stream::XlsxStream;
use super::{BODY_PART_ID, TEXT_PART_ID};
use crate::codec::ooxml::{OoxmlAddress, blob_replacements, replacements, without_removed};
use crate::codec::props::DocProps;
use crate::opc::check::check;
use crate::xlsx::{SheetClassifier, Xlsx};

/// A decoded workbook: its cells, its text outside the cells (comments,
/// drawings, charts, external hyperlink targets), and its document-property
/// streams over the retained package.
///
/// [`encode`](Native::encode) turns the redacted cells into
/// [`CellEdit`](crate::xlsx::CellEdit)s, the redacted non-cell text into text
/// splices, and the property streams and blobs (embeddings) into part
/// replacements, for one byte-faithful re-pack.
pub(super) struct XlsxDocument {
    /// The workbook's text-bearing cells.
    pub(super) cells: XlsxStream,
    /// The workbook's text outside its cells.
    pub(super) text: ExtractStream<OoxmlAddress>,
    /// The document-property parts, when metadata is treated as content.
    pub(super) properties: DocProps,
    /// The (sanitized) package bytes, retained so the engine re-packs every
    /// unedited part unchanged.
    pub(super) archive: Bytes,
    /// The policy the decode sanitized under, which the output must pass.
    pub(super) policy: HiddenContentPolicy,
}

impl Native for XlsxDocument {
    fn streams(&mut self) -> Vec<(LocalId, StreamMut<'_>)> {
        let cells = (
            LocalId::new(BODY_PART_ID),
            StreamMut::new::<Tabular, _>(&mut self.cells),
        );
        let text = (
            LocalId::new(TEXT_PART_ID),
            StreamMut::new::<Text, _>(&mut self.text),
        );
        iter::once(cells)
            .chain(iter::once(text))
            .chain(self.properties.streams())
            .collect()
    }

    fn encode(&self, blobs: &[Blob], removed: &[LocalId]) -> Result<ContentData> {
        let edits = self.cells.state.cell_edits();
        let text = replacements(self.text.items());
        let mut parts = self.properties.replacements();
        parts.extend(blob_replacements(blobs));
        let bytes =
            Xlsx::open(&self.archive).and_then(|xlsx| xlsx.rewrite(&edits, &text, &parts))?;
        let bytes = without_removed(bytes, removed)?;
        Ok(ContentData::new(Bytes::from(bytes)))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        check(output.as_bytes(), self.policy, &SheetClassifier)
    }
}
