//! The result of [`Pdf::extract`](crate::document::Pdf::extract): the [`Extraction`] and
//! its per-page text [`Block`]s, the [`Embedding`]s it surfaces for image
//! redaction, and the [`Issue`]s recording pages it could not read.

use std::collections::HashSet;

use bytes::Bytes;
use hipstr::HipStr;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::document::Store;
use crate::document::image::{EmbeddingKind, ImageId};
use crate::text::text_block_for_page;

/// The result of [`Pdf::extract`](crate::document::Pdf::extract): per-page text blocks,
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
    #[cfg(feature = "render")]
    #[must_use]
    pub fn new(page: u32, text: impl Into<HipStr<'static>>) -> Self {
        Self {
            page,
            text: text.into(),
        }
    }
}

impl Extraction {
    /// Extract the document in `store`, returning the [`Extraction`] and each
    /// readable page's rendered glyph [`geometry`](crate::text::PageGeometry) from
    /// one content walk, so the codec maps a detected span to page rectangles
    /// without walking every page twice. A page the walk cannot read contributes
    /// no geometry (and is flagged as an issue), but the readable pages' geometry
    /// is retained.
    ///
    /// A [`Block`] is addressed by its 1-based [`page`](Block::page) and its
    /// [`text`](Block::text), not a byte span, because PDF text lives in
    /// content-stream operators. An [`Embedding`] is addressed by its image
    /// object [`id`](Embedding::id).
    pub(crate) fn from_store_with_geometry(
        store: &Store,
    ) -> (Self, Vec<crate::text::PageGeometry>) {
        let doc = store.doc();
        let pages_map = doc.get_pages();
        let mut blocks = Vec::new();
        let mut embeddings = Vec::new();
        let mut issues = Vec::new();
        let mut geometry = Vec::new();
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
                Ok(block) if !block.text.trim().is_empty() => {
                    // Retain the page's glyph geometry from this one walk.
                    geometry.push(crate::text::PageGeometry {
                        page,
                        rects: block.rects,
                        frame: block.frame,
                    });
                    blocks.push(Block {
                        page,
                        text: HipStr::from(block.text),
                    });
                }
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

        (
            Self {
                blocks,
                embeddings,
                issues,
            },
            geometry,
        )
    }
}

/// One embedded image surfaced for redaction, addressed by its object id.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Embedding {
    /// The image XObject's object id.
    pub id: ImageId,
    /// The 1-based page the image appears on.
    pub page: u32,
    /// How the image is encoded.
    pub kind: EmbeddingKind,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// The image's raw stream bytes (a cheap ref-counted share).
    pub bytes: Bytes,
}

/// A page that [`Pdf::extract`](crate::document::Pdf::extract) recovered no text from, so
/// its text is **not** covered by the extracted blocks.
///
/// Extraction is partial-success: a page that yields no text does not fail the
/// whole document. An `Issue` records the gap and *why*, so a caller does not
/// silently treat a scanned or unreadable page as fully redacted, the dangerous
/// failure mode for redaction. A document whose every page yields text produces
/// none.
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
