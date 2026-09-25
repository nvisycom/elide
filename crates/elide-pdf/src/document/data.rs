//! [`Store`]: the opened-document data holder the capabilities operate over.
//!
//! `Store` owns the parsed object graph and the pristine source bytes, plus the
//! decompression bound, opened once and reused. It carries no
//! extraction/inspection/redaction logic; those capabilities are functions in
//! their own modules that take a `&Store` (or `&mut Store`) and read the object
//! graph through it. The public [`Pdf`](super::Pdf) is a thin orchestrator over
//! one `Store`.

use std::num::NonZeroUsize;

#[cfg(feature = "render")]
use bytes::Bytes;
use elide_core::{Error, ErrorKind, Result};
use lopdf::Document;

/// The opened-document data holder: the parsed object graph, the source bytes,
/// and the per-page decompression bound.
#[derive(Debug, Clone)]
pub(crate) struct Store {
    /// The parsed object graph. Capabilities read and clone it through the
    /// crate-visible accessors below.
    doc: Document,
    /// The original bytes the document was opened from, retained so the `render`
    /// feature can rasterise the pristine PDF rather than a lossy lopdf
    /// re-serialisation.
    #[cfg(feature = "render")]
    source: Bytes,
    /// Bound on a single page's decompressed content, guarding against a
    /// decompression bomb.
    max_page_bytes: NonZeroUsize,
}

impl Store {
    /// Default bound on a single page's decompressed content.
    pub(crate) const DEFAULT_MAX_PAGE_BYTES: NonZeroUsize =
        NonZeroUsize::new(64 * 1024 * 1024).expect("non-zero");
    /// Maximum accepted source-document size (64 MiB).
    pub(crate) const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

    /// Parse `document` under `max_page_bytes`, retaining the source bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](crate::ErrorKind::ResourceLimit) if the input
    /// exceeds [`MAX_DOCUMENT_BYTES`](Self::MAX_DOCUMENT_BYTES), or
    /// [`ErrorKind::MalformedInput`](crate::ErrorKind::MalformedInput) if the
    /// bytes are not a readable PDF.
    pub(crate) fn open(document: &[u8], max_page_bytes: NonZeroUsize) -> Result<Self> {
        if document.len() > Self::MAX_DOCUMENT_BYTES {
            return Err(Error::new(
                ErrorKind::ResourceLimit,
                format!(
                    "document is {} bytes, over the {}-byte limit",
                    document.len(),
                    Self::MAX_DOCUMENT_BYTES
                ),
            ));
        }
        let doc = Document::load_mem(document).map_err(|e| {
            Error::new(
                ErrorKind::MalformedInput,
                format!("not a readable PDF: {e}"),
            )
        })?;
        Ok(Self {
            doc,
            #[cfg(feature = "render")]
            source: Bytes::copy_from_slice(document),
            max_page_bytes,
        })
    }

    /// The parsed object graph.
    pub(crate) fn doc(&self) -> &Document {
        &self.doc
    }

    /// A fresh clone of the object graph, for a capability that mutates a copy
    /// (redaction clones, edits, and re-serialises, leaving the store intact).
    pub(crate) fn clone_doc(&self) -> Document {
        self.doc.clone()
    }

    /// The pristine bytes the document was opened from.
    #[cfg(feature = "render")]
    pub(crate) fn source_bytes(&self) -> Bytes {
        self.source.clone()
    }

    /// The per-page decompression bound.
    pub(crate) fn max_page_bytes(&self) -> NonZeroUsize {
        self.max_page_bytes
    }
}
