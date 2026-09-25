//! [`XlsxRecombine`]: re-pack the workbook from the redacted cells and blob parts.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::{EncodedPart, Recombine};
use elide_core::Result;

use super::BODY_PART_ID;
use super::state::XlsxState;
use crate::xlsx::Xlsx;

/// Re-packs the workbook: the redacted cells (from the shared [`XlsxState`])
/// become [`CellEdit`](crate::xlsx::CellEdit)s and the redacted text/property
/// parts (from the blob [`EncodedPart`]s) fold in alongside for one byte-faithful
/// re-pack.
pub(super) struct XlsxRecombine {
    /// The original package bytes, retained so the engine re-packs every unedited
    /// part unchanged.
    pub(super) archive: Bytes,
    /// The shared cell state, read to build the cell edits.
    pub(super) state: XlsxState,
}

impl Recombine for XlsxRecombine {
    fn assemble(&self, parts: &[EncodedPart]) -> Result<ContentData> {
        // Rewrite only the cells a redaction actually changed. Sending every cell
        // would de-share the whole workbook (each shared cell becomes an inline
        // string); sending just the edited ones de-shares only what was redacted
        // and leaves the shared-string table otherwise intact.
        let edits = self.state.cell_edits();
        // The redacted non-cell text parts (comments, drawings, charts) and
        // document-property parts fold in alongside the cell edits. Each blob's id
        // is its zip entry path; the body stream part carries no zip entry.
        let part_edits: Vec<(String, Vec<u8>)> = parts
            .iter()
            .filter(|p| p.id.as_str() != BODY_PART_ID)
            .map(|p| (p.id.as_str().to_owned(), p.bytes.to_vec()))
            .collect();
        let bytes = Xlsx::open(&self.archive)
            .and_then(|xlsx| xlsx.rewrite_with_parts(&edits, &part_edits))?;
        Ok(ContentData::new(Bytes::from(bytes)))
    }
}
