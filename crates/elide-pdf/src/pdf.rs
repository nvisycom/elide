//! [`Pdf`]: an opened PDF document — extract its text and images, inspect its
//! structure, and redact it.

use std::collections::HashSet;
use std::num::NonZeroUsize;

use bytes::Bytes;
use hipstr::HipStr;
use lopdf::Document;

use crate::error::{Error, Result};
use crate::extract::{Block, Embedding, EmbeddingKind, Extraction, ImageId, Issue, IssueKind};

/// An opened PDF document: parsed once, ready to [`extract`](Pdf::extract) its
/// text, [`inspect`](Pdf::inspect) its structure, or redact it — by
/// [`redact_text`](Pdf::redact_text) (delete glyphs, keep a selectable layer) or,
/// with the `render` feature, `redact_raster` (flatten to images).
///
/// Open a document once and reuse it across operations.
#[derive(Debug, Clone)]
pub struct Pdf {
    pub(crate) doc: Document,
    max_page_bytes: NonZeroUsize,
    /// The original bytes this document was opened from, retained so
    /// [`inspect`](Pdf::inspect) can account for retained/superseded bytes and
    /// the `render` feature can rasterise the pristine PDF rather than a lossy
    /// lopdf re-serialisation.
    source: Bytes,
}

impl Pdf {
    /// Default bound on a single page's decompressed content, guarding against
    /// a decompression bomb. Used by [`open`](Pdf::open); override it with
    /// [`open_with_limit`](Pdf::open_with_limit).
    pub const DEFAULT_MAX_PAGE_BYTES: NonZeroUsize =
        NonZeroUsize::new(64 * 1024 * 1024).expect("non-zero");
    /// Maximum accepted source-document size (64 MiB). A larger input is
    /// refused by [`open`](Pdf::open) before parsing.
    pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

    /// Open a PDF from its bytes, using the [default page
    /// bound](Pdf::DEFAULT_MAX_PAGE_BYTES).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if the
    ///   input exceeds [`MAX_DOCUMENT_BYTES`](Pdf::MAX_DOCUMENT_BYTES);
    /// - [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if the
    ///   bytes are not a readable PDF.
    pub fn open(document: &[u8]) -> Result<Self> {
        Self::open_with_limit(document, Self::DEFAULT_MAX_PAGE_BYTES)
    }

    /// Open a PDF with an explicit per-page decompressed-size bound.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::LimitExceeded`](crate::ErrorKind::LimitExceeded) if the
    ///   input exceeds [`MAX_DOCUMENT_BYTES`](Pdf::MAX_DOCUMENT_BYTES);
    /// - [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if the
    ///   bytes are not a readable PDF.
    pub fn open_with_limit(document: &[u8], max_page_bytes: NonZeroUsize) -> Result<Self> {
        if document.len() > Self::MAX_DOCUMENT_BYTES {
            return Err(Error::limit_exceeded(format!(
                "document is {} bytes, over the {}-byte limit",
                document.len(),
                Self::MAX_DOCUMENT_BYTES
            )));
        }
        let doc = Document::load_mem(document)
            .map_err(|e| Error::invalid_document(format!("not a readable PDF: {e}")))?;
        Ok(Self {
            doc,
            max_page_bytes,
            source: Bytes::copy_from_slice(document),
        })
    }

    /// The original bytes this document was opened from.
    pub(crate) fn source_bytes(&self) -> Bytes {
        self.source.clone()
    }

    /// Extract the text and embedded images of the PDF, with any per-page
    /// [`issues`](Extraction::issues) for pages that yielded no text.
    ///
    /// A [`Block`] is addressed by its 1-based [`page`](Block::page) and its
    /// [`text`](Block::text) — not a byte span, because PDF text lives in
    /// content-stream operators. An [`Embedding`] is addressed by its image
    /// object [`id`](Embedding::id).
    pub fn extract(&self) -> Extraction {
        let pages_map = self.doc.get_pages();
        let mut blocks = Vec::new();
        let mut embeddings = Vec::new();
        let mut issues = Vec::new();
        // An image XObject shared across pages is surfaced once, addressed by
        // the first (lowest-numbered) page it appears on.
        let mut seen_images = HashSet::new();

        for (&page, &page_id) in &pages_map {
            // Partial-success: a page with no text layer or an unreadable page
            // is recorded as an issue rather than failing the whole extraction.
            match self
                .doc
                .extract_text_with_limit(&[page], self.max_page_bytes.get())
            {
                Ok(text) if !text.trim().is_empty() => blocks.push(Block {
                    page,
                    text: HipStr::from(text),
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

            // The page's embedded image XObjects, surfaced for redaction. A
            // page with no images (or an unreadable image tree) simply adds
            // none; it does not fail the whole extraction.
            if let Ok(images) = self.doc.get_page_images(page_id) {
                for image in images {
                    let id = ImageId::from_object(image.id);
                    if !seen_images.insert(id) {
                        continue;
                    }
                    // Dimensions that are negative or exceed `u32::MAX` are not
                    // a real raster to redact; skip the image rather than coerce
                    // a bogus zero/truncated size into the extraction.
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

        Extraction {
            blocks,
            embeddings,
            issues,
        }
    }
}
