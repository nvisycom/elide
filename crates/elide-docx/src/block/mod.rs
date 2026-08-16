//! The extraction and redaction units: the [`Extraction`] result and its
//! [`Block`]s, [`Embedding`]s, and [`Issue`]s, plus the [`Replacement`] and
//! [`PartReplacement`] applied on rewrite — each addressed by a typed
//! [`PartPath`].

mod offset;
mod replacement;

use std::ops::Range;

use bytes::Bytes;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::offset::{OffsetMap, OffsetRun};
pub use self::replacement::{PartReplacement, Replacement};
use crate::part::{EmbeddingKind, PartPath};

/// The result of [`Docx::extract`](crate::Docx::extract): the redactable text
/// [`blocks`](Extraction::blocks) of every text-bearing part, the binary
/// [`embeddings`](Extraction::embeddings) surfaced for redaction, and any
/// [`issues`](Extraction::issues) that left a part un-extracted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Extraction {
    /// The redactable text blocks, in part-then-document order. Each carries the
    /// part it came from and its byte span within that part.
    pub blocks: Vec<Block>,
    /// The binary embeddings (images, objects, fonts) surfaced for redaction.
    pub embeddings: Vec<Embedding>,
    /// The text-bearing parts that could not be extracted. Empty on a clean
    /// extraction; a non-empty list means some part's text is not covered by
    /// the blocks.
    pub issues: Vec<Issue>,
}

/// One redactable unit of text, addressed by the part it came from and its
/// byte span within that part's XML.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Block {
    /// The package part this text is in.
    pub part: PartPath,
    /// The block's logical text, with XML entities decoded (e.g. `&amp;` reads
    /// as `&`), so recognizers match the text a reader sees. The byte
    /// [`span`](Block::span) still addresses the raw source range.
    pub text: HipStr<'static>,
    /// Start of the byte range within the part's XML.
    pub start: usize,
    /// End of the byte range (exclusive).
    pub end: usize,
    /// The decoded-to-raw byte correspondence: how a byte offset into
    /// [`text`](Block::text) maps back to a part-absolute raw byte range,
    /// accounting for entity substitutions. Its raw offsets are absolute in the
    /// part, so they line up with [`span`](Block::span).
    pub offsets: OffsetMap,
}

impl Block {
    /// The block's byte range within its part's XML.
    pub fn span(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// One binary embedding surfaced for redaction (an image, embedded object, or
/// font), addressed by its part.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Embedding {
    /// The package part holding the embedding.
    pub part: PartPath,
    /// What kind of embedding it is.
    pub kind: EmbeddingKind,
    /// The embedding's raw bytes (a cheap share of the stored part buffer).
    pub bytes: Bytes,
}

/// A text-bearing part that [`Docx::extract`](crate::Docx::extract) could not
/// read, so its text is **not** covered by the extracted blocks.
///
/// Extraction is partial-success: a corrupt or non-UTF-8 part does not fail the
/// whole document, but it also yields no blocks. An `Issue` records that gap so
/// a caller does not silently ship a document with an un-redacted part — the
/// dangerous failure mode for redaction. A clean extraction produces none.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Issue {
    /// The part that could not be extracted.
    pub part: PartPath,
    /// Why it could not be extracted.
    pub kind: IssueKind,
}

/// Why a text-bearing part could not be extracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum IssueKind {
    /// The part's bytes are not valid UTF-8.
    NotUtf8,
    /// The part's XML could not be parsed.
    MalformedXml,
}
