//! The extraction and redaction units: the [`Extraction`] result and its
//! [`Block`]s, [`Embedding`]s, and [`Issue`]s, plus the [`Replacement`] and
//! [`PartReplacement`] applied on rewrite, each addressed by a typed
//! [`PartPath`].

use std::ops::Range;

use bytes::Bytes;
use elide_codec::extract::{ItemFlag, OffsetMap, Passage};
use hipstr::HipStr;

use crate::opc::part::PartPath;

/// The result of extracting a package: the redactable text
/// [`blocks`](Extraction::blocks) of every text-bearing part, the binary
/// [`embeddings`](Extraction::embeddings) surfaced for redaction, and any
/// [`issues`](Extraction::issues) that left a part un-extracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Extraction {
    /// The redactable text blocks, in part-then-document order. Each carries the
    /// part it came from and its byte span within that part.
    pub(crate) blocks: Vec<Block>,
    /// The binary embeddings (images, audio, video, objects) surfaced for
    /// redaction.
    pub(crate) embeddings: Vec<Embedding>,
    /// The text-bearing parts that could not be extracted. Empty on a clean
    /// extraction; a non-empty list means some part's text is not covered by
    /// the blocks.
    pub(crate) issues: Vec<Issue>,
}

#[cfg(test)]
impl Extraction {
    /// The text of every block, in order, for a test asserting on it.
    pub(crate) fn texts(&self) -> Vec<String> {
        self.blocks
            .iter()
            .map(|block| block.text.to_string())
            .collect()
    }
}

/// One redactable unit of text, addressed by the part it came from and its
/// byte span within that part's XML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    /// The package part this text is in.
    pub(crate) part: PartPath,
    /// The block's logical text, with XML entities decoded (e.g. `&amp;` reads
    /// as `&`), so recognizers match the text a reader sees. The byte range
    /// [`span`](Block::span) still addresses the raw source.
    pub(crate) text: HipStr<'static>,
    /// The byte range within the part's XML.
    pub(crate) span: Range<usize>,
    /// The decoded-to-raw byte correspondence: how a byte offset into
    /// [`text`](Block::text) maps back to a part-absolute raw byte range,
    /// accounting for entity substitutions. Its raw offsets are absolute in the
    /// part, so they line up with [`span`](Block::span).
    pub(crate) offsets: OffsetMap,
    /// How the block joins the passage of the block before it.
    pub(crate) passage: Passage,
    /// What the block is when it is collaboration content (a comment, its
    /// author, a tracked deletion); `None` for visible content.
    pub(crate) flag: Option<ItemFlag>,
}

/// The kind of binary embedding a part holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EmbeddingKind {
    /// An embedded image (e.g. an image in `word/media/*`).
    Image,
    /// An embedded audio clip (e.g. an `.mp3`/`.wav` in `ppt/media/*`).
    Audio,
    /// An embedded video clip (e.g. an `.mp4`/`.mov` in `ppt/media/*`).
    Video,
    /// An embedded object / OLE package (e.g. `word/embeddings/*`).
    Object,
}

/// One binary embedding surfaced for redaction (an image, audio or video clip,
/// or embedded object), addressed by its part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Embedding {
    /// The package part holding the embedding.
    pub(crate) part: PartPath,
    /// What kind of embedding it is.
    pub(crate) kind: EmbeddingKind,
    /// The embedding's raw bytes (a cheap share of the stored part buffer).
    pub(crate) bytes: Bytes,
}

/// A text-bearing part that extraction could not read, so its text is **not**
/// covered by the extracted blocks.
///
/// Extraction is partial-success: a corrupt or non-UTF-8 part does not fail the
/// whole document, but it also yields no blocks. An `Issue` records that gap so
/// a caller does not silently ship a document with an un-redacted part, the
/// dangerous failure mode for redaction. A clean extraction produces none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Issue {
    /// The part that could not be extracted.
    pub(crate) part: PartPath,
    /// Why it could not be extracted.
    pub(crate) kind: IssueKind,
}

/// Why a text-bearing part could not be extracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum IssueKind {
    /// The part's bytes are not valid UTF-8.
    NotUtf8,
    /// The part's XML could not be parsed.
    MalformedXml,
}

/// One text replacement: overwrite the bytes `span` of `part`'s XML with
/// `text`.
///
/// The span is a byte range into the named part's XML, as carried on a
/// [`Block`] from the same part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replacement {
    /// The part to rewrite.
    pub(crate) part: PartPath,
    /// The byte range to overwrite.
    pub(crate) span: Range<usize>,
    /// The text to write in place of the span.
    pub(crate) text: HipStr<'static>,
}

/// One binary part replacement: overwrite `part`'s bytes with `bytes` (e.g. a
/// redacted image).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartReplacement {
    /// The part to replace.
    pub(crate) part: PartPath,
    /// The new bytes for the part.
    pub(crate) bytes: Bytes,
}

impl PartReplacement {
    /// A replacement overwriting `part`'s bytes with `bytes`.
    pub(crate) fn new(part: PartPath, bytes: impl Into<Bytes>) -> Self {
        Self {
            part,
            bytes: bytes.into(),
        }
    }
}
