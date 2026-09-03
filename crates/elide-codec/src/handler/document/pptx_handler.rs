//! PPTX handler side: adapts the [`elide_office`] presentation engine to the
//! codec's [`Handler`] contract.
//!
//! Like DOCX, a PPTX is a container of parts: slide (and notes, master, comment)
//! text plus embedded media in `ppt/media/*`. The handler *is* an
//! [`ExtractHandler`] over the text blocks [`Pptx::extract`](elide_office::pptx::Pptx::extract)
//! recovers; on encode, [`PptxEncoder`] turns the edits into
//! [`elide_office::opc::Replacement`]s (plus any redacted media parts) and calls
//! [`Pptx::rewrite_with_parts`](elide_office::pptx::Pptx::rewrite_with_parts),
//! which owns the package round-trip.
//!
//! [`ExtractHandler`]: crate::handler::extract::ExtractHandler

use std::collections::HashMap;
use std::ops::Range;

use bytes::Bytes;
use elide_core::modality::text::{SourceRef, Text};
use elide_core::{Error, ErrorKind, Result};
use elide_office::opc::{Embedding, OffsetMap, PartPath};
use elide_office::pptx::PartKind;

use super::PptxLoader;
use crate::codec::{Container, Part};
use crate::content::ContentData;
use crate::handler::extract::{Encoder, ExtractHandler, ExtractedItem, ItemEdit};
use crate::{Format, FormatId, LocalId};

/// Stable [`FormatId`] for the PPTX codec.
pub const FORMAT_ID: FormatId = FormatId::new("elide.document.pptx");

/// Handler type for loaded PPTX content.
pub(crate) type PptxHandler = ExtractHandler<PptxEncoder>;

/// The address of a PPTX text block: which package part it is in, and its byte
/// span within that part's XML.
#[derive(Debug, Clone)]
pub(crate) struct PptxAddress {
    /// The part the block belongs to.
    pub(crate) part: PartPath,
    /// The block's byte span within the part's XML.
    pub(crate) span: Range<usize>,
    /// The block's decoded-to-raw offset map, so a byte range into the decoded
    /// value can be translated back to its exact raw source range(s).
    pub(crate) offsets: OffsetMap,
}

/// [`Format`] descriptor registered into [`FormatRegistry`].
///
/// [`FormatRegistry`]: crate::FormatRegistry
pub fn format() -> Format {
    Format::new::<Text, _>(FORMAT_ID.clone(), PptxLoader)
        .with_extensions(["pptx"])
        .with_content_types([
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ])
}

/// Re-packs a PPTX by delegating to
/// [`Pptx::rewrite_with_parts`](elide_office::pptx::Pptx::rewrite_with_parts):
/// the redacted text blocks become text replacements and any redacted media
/// parts travel alongside.
#[derive(Debug)]
pub(crate) struct PptxEncoder {
    /// The original package bytes, retained so [`elide_office`] can re-pack every
    /// unredacted part unchanged.
    pub(super) archive: Bytes,
    /// The binary embeddings surfaced for redaction, cached at decode so the
    /// [`Container`] surface lists them and validates ids without re-extracting.
    pub(super) embeddings: Vec<Embedding>,
    /// Redacted replacements for media parts, keyed by zip entry name, filled
    /// through the [`Container`] surface.
    pub(super) replacements: HashMap<String, Bytes>,
}

impl Encoder for PptxEncoder {
    type Address = PptxAddress;

    fn encode(&self, items: &[ExtractedItem<PptxAddress>]) -> Result<ContentData> {
        // Each item's (current) value overwrites its source byte span in its
        // part's XML. `elide_office` validates and applies these fail-closed.
        let text_replacements: Vec<elide_office::opc::Replacement> = items
            .iter()
            .map(|item| elide_office::opc::Replacement {
                part: item.address.part.clone(),
                start: item.address.span.start,
                end: item.address.span.end,
                text: item.value.clone().into(),
            })
            .collect();
        let media: Vec<elide_office::opc::PartReplacement> = self
            .replacements
            .iter()
            .map(|(name, bytes)| {
                elide_office::opc::PartReplacement::new(PartPath::new(name.clone()), bytes.to_vec())
            })
            .collect();

        let out = elide_office::pptx::Pptx::open(&self.archive)
            .and_then(|pptx| pptx.rewrite_with_parts(&text_replacements, &media))
            .map_err(pptx_error)?;
        Ok(ContentData::new(Bytes::from(out)))
    }

    fn source_span(
        &self,
        item: &ExtractedItem<PptxAddress>,
        local: Range<usize>,
    ) -> Vec<SourceRef> {
        // Translate the decoded-value range back to its raw source range(s) via
        // the block's offset map (already part-absolute), tagging each with the
        // part it lives in. A range crossing an entity yields several runs.
        super::opc_source::source_span(item.address.part.as_str(), &item.address.offsets, local)
    }

    fn locate_source(
        &self,
        items: &[ExtractedItem<PptxAddress>],
        source: &[SourceRef],
    ) -> Option<ItemEdit> {
        super::opc_source::locate_source(
            items.iter().map(|item| {
                (
                    item.address.part.as_str(),
                    item.address.span.clone(),
                    &item.address.offsets,
                )
            }),
            source,
        )
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        Some(self)
    }
}

impl Container for PptxEncoder {
    fn parts(&self) -> Vec<Part> {
        // Surface every binary embedding the engine classifies, images and
        // media under `ppt/media/`, embedded objects under `ppt/embeddings/`.
        self.embeddings
            .iter()
            .map(|embedding| {
                let id = LocalId::new(embedding.part.as_str().to_owned());
                let hint = id.extension().unwrap_or_default().to_owned();
                Part {
                    id,
                    bytes: embedding.bytes.clone(),
                    hint,
                }
            })
            .collect()
    }

    fn replace_part(&mut self, id: &LocalId, bytes: Bytes) -> Result<()> {
        // Reject anything that isn't a binary embedding so a caller can't smuggle
        // bytes into a text/structure part through this surface.
        if PartKind::of(&PartPath::from(id.as_str()))
            .embedding()
            .is_none()
        {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("pptx replace_part: `{id}` is not an embedded media part"),
            ));
        }
        // And reject ids that name no embedding the presentation actually
        // carries, an unknown id must not be silently stored and dropped.
        let is_known = self
            .embeddings
            .iter()
            .any(|embedding| embedding.part.as_str() == id.as_str());
        if !is_known {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!("pptx replace_part: `{id}` is not a known embedded media part"),
            ));
        }
        self.replacements.insert(id.as_str().to_owned(), bytes);
        Ok(())
    }
}

/// Map an [`elide_office`] error into the codec's error type.
pub(super) fn pptx_error(err: elide_office::Error) -> Error {
    use elide_office::ErrorKind as OfficeKind;
    let kind = match err.kind() {
        OfficeKind::InvalidArchive | OfficeKind::InvalidPackage | OfficeKind::InvalidXml => {
            ErrorKind::MalformedInput
        }
        OfficeKind::UnsafeRewrite => ErrorKind::Processing,
        _ => ErrorKind::Processing,
    };
    Error::new(kind, err.to_string())
}
