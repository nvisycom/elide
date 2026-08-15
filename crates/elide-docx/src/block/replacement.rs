//! [`Replacement`] and [`PartReplacement`]: the redaction inputs, each
//! addressed by a typed [`PartPath`].

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use super::Block;
use crate::part::PartPath;

/// One text replacement: overwrite the bytes `[start, end)` of `part`'s XML
/// with `text`.
///
/// The span is a byte range into the named part's XML, as carried on a
/// [`Block`] from the same part.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Replacement {
    /// The part to rewrite.
    pub part: PartPath,
    /// Start of the byte range to overwrite.
    pub start: usize,
    /// End of the byte range to overwrite (exclusive).
    pub end: usize,
    /// The text to write in place of the span.
    pub text: HipStr<'static>,
}

impl Replacement {
    /// A replacement overwriting `block`'s span with `text`.
    pub fn for_block(block: &Block, text: impl Into<HipStr<'static>>) -> Self {
        Self {
            part: block.part.clone(),
            start: block.start,
            end: block.end,
            text: text.into(),
        }
    }
}

/// One binary part replacement: overwrite `part`'s bytes with `bytes` (e.g. a
/// redacted image).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PartReplacement {
    /// The part to replace.
    pub part: PartPath,
    /// The new bytes for the part.
    pub bytes: Vec<u8>,
}
