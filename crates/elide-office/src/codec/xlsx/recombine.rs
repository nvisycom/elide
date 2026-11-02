//! [`XlsxRecombine`]: re-pack the workbook from the redacted cells and blob
//! parts, then check the package it produced.

use bytes::Bytes;
use elide_codec::content::ContentData;
use elide_codec::extract::SharedSplice;
use elide_codec::{EncodedPart, LocalId, Recombine};
use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::state::XlsxState;
use super::{BODY_PART_ID, TEXT_PART_ID};
use crate::codec::ooxml::{OoxmlAddress, replacements, without_removed};
use crate::opc::check::check;
use crate::opc::{PartPath, PartReplacement};
use crate::xlsx::{SheetClassifier, Xlsx};

/// Re-packs the workbook: the redacted cells (from the shared [`XlsxState`])
/// become [`CellEdit`](crate::xlsx::CellEdit)s, the redacted non-cell text
/// (from the shared text items) becomes text splices, and the redacted
/// embedding and property parts (from the blob [`EncodedPart`]s) fold in
/// alongside, for one byte-faithful re-pack.
pub(super) struct XlsxRecombine {
    /// The (sanitized) package bytes, retained so the engine re-packs every
    /// unedited part unchanged.
    pub(super) archive: Bytes,
    /// The shared cell state, read to build the cell edits.
    pub(super) state: XlsxState,
    /// The (redacted-in-place) non-cell text items, shared with the text stream.
    pub(super) text: SharedSplice<OoxmlAddress>,
    /// The policy the decode sanitized under, which the output must pass.
    pub(super) policy: HiddenContentPolicy,
}

impl Recombine for XlsxRecombine {
    fn assemble(&self, parts: &[EncodedPart], removed: &[LocalId]) -> Result<ContentData> {
        // Rewrite only the cells a redaction actually changed. Sending every cell
        // would de-share the whole workbook (each shared cell becomes an inline
        // string); sending just the edited ones de-shares only what was redacted
        // and leaves the shared-string table otherwise intact.
        let edits = self.state.cell_edits();
        let text = self.text.with_items(replacements);
        // Each embedding's and property part's id is its zip entry path; the
        // cell and text streams carry none.
        let blobs: Vec<PartReplacement> = parts
            .iter()
            .filter(|p| p.id.as_str() != BODY_PART_ID && p.id.as_str() != TEXT_PART_ID)
            .map(|p| PartReplacement::new(PartPath::from(p.id.as_str()), p.bytes.to_vec()))
            .collect();
        let bytes =
            Xlsx::open(&self.archive).and_then(|xlsx| xlsx.rewrite(&edits, &text, &blobs))?;
        let bytes = without_removed(bytes, removed)?;
        Ok(ContentData::new(Bytes::from(bytes)))
    }

    fn validate(&self, output: &ContentData) -> Result<()> {
        check(output.as_bytes(), self.policy, &SheetClassifier)
    }
}
