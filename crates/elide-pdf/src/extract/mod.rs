//! The result of [`Pdf::extract`](crate::Pdf::extract): the [`Extraction`] and
//! its per-page text [`Block`]s, the [`Embedding`]s it surfaces for image
//! redaction, and the [`Issue`]s recording pages it could not read.

mod embedding;
mod issue;

use std::collections::HashSet;

use bytes::Bytes;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub use self::embedding::{Embedding, EmbeddingKind, ImageId};
pub use self::issue::{Issue, IssueKind};
use crate::document::Store;
use crate::text::text_block_for_page;

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

impl Block {
    /// A block of `text` on 1-based `page`.
    #[must_use]
    pub fn new(page: u32, text: impl Into<HipStr<'static>>) -> Self {
        Self {
            page,
            text: text.into(),
        }
    }
}

impl Extraction {
    /// Extract the text and embedded images of the document in `store`, with any
    /// per-page [`issues`](Extraction::issues) for pages that yielded no text.
    ///
    /// A [`Block`] is addressed by its 1-based [`page`](Block::page) and its
    /// [`text`](Block::text), not a byte span, because PDF text lives in
    /// content-stream operators. An [`Embedding`] is addressed by its image
    /// object [`id`](Embedding::id).
    pub(crate) fn from_store(store: &Store) -> Self {
        let doc = store.doc();
        let pages_map = doc.get_pages();
        let mut blocks = Vec::new();
        let mut embeddings = Vec::new();
        let mut issues = Vec::new();
        // An image XObject shared across pages is surfaced once, addressed by the
        // first (lowest-numbered) page it appears on.
        let mut seen_images = HashSet::new();

        for (&page, &page_id) in &pages_map {
            // Walk the page with the same engine redaction uses, so the extracted
            // text (and the offsets a detection indexes) align with what a
            // redaction later locates. Partial-success: a page with no text layer
            // is flagged for OCR and a page the walk cannot decode is flagged
            // unreadable, rather than failing the whole extraction, the redactor
            // (which must not silently skip) fails closed on the same page.
            match text_block_for_page(doc, page, page_id, store.max_page_bytes().get()) {
                Ok(block) if !block.text.trim().is_empty() => blocks.push(Block {
                    page,
                    text: HipStr::from(block.text),
                }),
                Ok(_) => issues.push(Issue {
                    page,
                    kind: IssueKind::NeedsOcr,
                }),
                Err(_) => issues.push(Issue {
                    page,
                    kind: IssueKind::Unreadable,
                }),
            }

            // The page's embedded image XObjects, surfaced for redaction. A page
            // with no images (or an unreadable image tree) simply adds none; it does
            // not fail the whole extraction.
            if let Ok(images) = doc.get_page_images(page_id) {
                for image in images {
                    let id = ImageId::from_object(image.id);
                    if !seen_images.insert(id) {
                        continue;
                    }
                    // Dimensions that are negative or exceed `u32::MAX` are not a
                    // real raster to redact; skip the image rather than coerce a
                    // bogus zero/truncated size into the extraction.
                    let (Ok(width), Ok(height)) =
                        (u32::try_from(image.width), u32::try_from(image.height))
                    else {
                        continue;
                    };
                    embeddings.push(Embedding {
                        id,
                        page,
                        kind: EmbeddingKind::from_filters(image.filters.as_deref()),
                        width,
                        height,
                        bytes: Bytes::copy_from_slice(image.content),
                    });
                }
            }
        }

        Self {
            blocks,
            embeddings,
            issues,
        }
    }
}
