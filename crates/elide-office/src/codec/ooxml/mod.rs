//! The shared OOXML codec layer: helpers common to the docx, pptx, and xlsx
//! adapters over this crate's package engine.
//!
//! The two text-only OOXML formats decode to the same
//! [`OoxmlPackage`](crate::ooxml::OoxmlPackage) and build the same
//! [`Document`](elide_codec::Document) the same way; this module holds the pieces
//! that would otherwise be copy-pasted across them: extract element-text blocks
//! addressed by `(part, span, OffsetMap)` into a body
//! [`ExtractStream`](elide_codec::extract::ExtractStream) (the
//! [`addresser`] maps decoded↔raw), surface embeddings as blobs and
//! document-property parts as metadata streams, and re-pack (the native
//! document, [`document`]). The only per-format differences — the engine format,
//! the codec [`FormatId`], and the label used in errors — are named by the
//! [`OoxmlCodec`] seam.

mod addresser;
mod document;
mod loader;

use std::collections::HashSet;
use std::ops::Range;

use elide_codec::extract::{ExtractedItem, OffsetMap, Passage};
use elide_codec::{Blob, FormatId, LocalId};
use elide_core::Result;

pub(crate) use self::addresser::OoxmlAddresser;
#[cfg(test)]
pub(crate) use self::document::OoxmlDocument;
pub(crate) use self::loader::OoxmlLoader;
#[cfg(test)]
pub(crate) use self::loader::decode_native;
use crate::ooxml::OoxmlFormat;
use crate::opc::sanitize::remove_parts;
use crate::opc::{Block, PartPath, PartReplacement, Replacement};

/// The document-local id of the body text stream (not a zip entry).
pub(crate) const BODY_PART_ID: &str = "body";

/// Ties a text-only OOXML engine format to its codec identity: the engine
/// format it opens, the stable [`FormatId`] it registers under, and the short
/// label used in its error messages (`docx`, `pptx`).
pub(crate) trait OoxmlCodec: Send + Sync + 'static {
    /// The engine format this codec drives.
    type Format: OoxmlFormat;

    /// The stable codec [`FormatId`].
    const FORMAT_ID: FormatId;

    /// A short lowercase name for the format, used in error messages.
    const LABEL: &'static str;
}

/// The address of an OOXML text block: which package part it is in, its byte
/// span within that part's XML, and the decoded-to-raw offset map that maps a
/// range into the decoded value back to its exact raw source range(s).
#[derive(Debug, Clone)]
pub(crate) struct OoxmlAddress {
    /// The part the block belongs to.
    pub(crate) part: PartPath,
    /// The block's byte span within the part's XML.
    pub(crate) span: Range<usize>,
    /// The block's decoded-to-raw offset map.
    pub(crate) offsets: OffsetMap,
}

/// The extracted items of an OOXML text stream: one per block, its value the
/// block's decoded text, addressed by its part, raw span, and offset map.
pub(crate) fn items(blocks: Vec<Block>) -> Vec<ExtractedItem<OoxmlAddress>> {
    blocks
        .into_iter()
        .map(|block| ExtractedItem {
            value: block.text.to_string(),
            address: OoxmlAddress {
                part: block.part,
                span: block.start..block.end,
                offsets: block.offsets,
            },
            hints: Vec::new(),
            passage: block.passage,
            flag: block.flag,
        })
        .collect()
}

/// Each blob's current bytes, as a replacement for the package part its id
/// names.
pub(crate) fn blob_replacements(blobs: &[Blob]) -> Vec<PartReplacement> {
    blobs
        .iter()
        .map(|blob| PartReplacement::new(PartPath::from(blob.id.as_str()), blob.bytes.to_vec()))
        .collect()
}

/// The text replacements that write each item's current value back over its
/// raw span; a separator is never written back.
pub(crate) fn replacements(items: &[ExtractedItem<OoxmlAddress>]) -> Vec<Replacement> {
    items
        .iter()
        .filter(|item| item.passage != Passage::Separator)
        .map(|item| Replacement {
            part: item.address.part.clone(),
            start: item.address.span.start,
            end: item.address.span.end,
            text: item.value.clone().into(),
        })
        .collect()
}

/// `package` without the blobs `removed` names (each id is its zip entry
/// path), along with every relationship and element that references one.
///
/// # Errors
///
/// [`MalformedInput`](elide_core::ErrorKind::MalformedInput) if the package
/// cannot be read or repacked.
pub(crate) fn without_removed(package: Vec<u8>, removed: &[LocalId]) -> Result<Vec<u8>> {
    let gone: HashSet<PartPath> = removed
        .iter()
        .map(|id| PartPath::from(id.as_str()))
        .collect();
    Ok(remove_parts(&package, gone)?.unwrap_or(package))
}
