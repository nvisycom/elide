//! The shared OOXML codec layer: helpers common to the docx and pptx adapters
//! over this crate's package engine.
//!
//! The two text-only OOXML formats decode to the same
//! [`OoxmlPackage`](crate::ooxml::OoxmlPackage) and build the same parts-model
//! [`Document`](elide_codec::Document) the same way; this module holds the pieces
//! that would otherwise be copy-pasted across them: extract element-text blocks
//! addressed by `(part, span, OffsetMap)` into a body
//! [`ExtractStream`](elide_codec::extract::ExtractStream) (the
//! [`addresser`](self::addresser) maps decoded↔raw), surface embeddings and
//! document-property parts as blob sub-parts, and re-pack (the
//! [`recombine`](self::recombine)r). The only per-format differences — the engine
//! format, the codec [`FormatId`], and the label used in errors — are named by
//! the [`OoxmlCodec`] seam.

mod addresser;
mod loader;
mod recombine;

use std::ops::Range;

use elide_codec::FormatId;

pub(crate) use self::loader::OoxmlLoader;
#[cfg(test)]
pub(crate) use self::loader::decode_parts;
#[cfg(test)]
pub(crate) use self::recombine::OoxmlRecombine;
use crate::ooxml::OoxmlFormat;
use crate::opc::{OffsetMap, PartPath};

/// The document-local id of the body text stream part (not a zip entry).
const BODY_PART_ID: &str = "body";

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
