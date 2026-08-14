//! The units of PDF extraction: the [`Extraction`] result and its [`Block`]s,
//! [`Issue`]s, and embedded-image types ([`Embedding`], [`ImageId`]).

mod image;

use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::image::{Embedding, EmbeddingKind, ImageId};

/// The result of [`Pdf::extract`](crate::Pdf::extract): per-page text blocks,
/// the embedded images surfaced for redaction, and any [`issues`](Extraction::issues)
/// for pages that yielded no text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Extraction {
    /// The recovered text blocks, in page order.
    pub blocks: Vec<Block>,
    /// The embedded images (XObjects) surfaced for redaction, in page order.
    pub embeddings: Vec<Embedding>,
    /// The pages that yielded no text (a scanned page needing OCR, or an
    /// unreadable page). Empty when every page yielded text.
    pub issues: Vec<Issue>,
}

/// One page's recovered text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Block {
    /// 1-based page number this text came from.
    pub page: u32,
    /// The page's extracted text.
    pub text: HipStr<'static>,
}

/// A page that [`Pdf::extract`](crate::Pdf::extract) recovered no text from, so
/// its text is **not** covered by the extracted blocks.
///
/// Extraction is partial-success: a page that yields no text does not fail the
/// whole document. An `Issue` records the gap and *why*, so a caller does not
/// silently treat a scanned or unreadable page as fully redacted — the
/// dangerous failure mode for redaction. A document whose every page yields
/// text produces none.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Issue {
    /// The 1-based page number.
    pub page: u32,
    /// Why the page yielded no text.
    pub kind: IssueKind,
}

/// Why a page yielded no extractable text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
#[non_exhaustive]
pub enum IssueKind {
    /// The page has no text layer: it is a scanned image, or its text is drawn
    /// as vector outlines. It needs OCR (see the `render` feature) rather than
    /// text redaction.
    NeedsOcr,
    /// The page's content could not be read (e.g. it exceeded the decompressed
    /// -size bound).
    Unreadable,
}
