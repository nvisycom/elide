//! [`Pdf`]: an opened PDF document, extracted and rewritten in place.

use std::collections::HashSet;
use std::num::NonZeroUsize;

use bytes::Bytes;
use hipstr::HipStr;
use lopdf::{Document, Object};

use crate::block::{Block, Extraction, Issue, IssueKind, Replacement};
use crate::error::{Error, Result};
use crate::image::{Embedding, EmbeddingKind, ImageId, ImageReplacement};

/// An opened PDF document: parsed once, ready to [`extract`](Pdf::extract) its
/// text or [`rewrite`](Pdf::rewrite) the born-digital text layer.
///
/// Open a document once and reuse it for both operations.
#[derive(Debug, Clone)]
pub struct Pdf {
    doc: Document,
    max_page_bytes: NonZeroUsize,
    /// The original bytes this document was opened from, retained so the
    /// `render` feature rasterises the pristine PDF rather than a lossy lopdf
    /// re-serialisation. Only needed by rendering, so only kept for it.
    #[cfg(feature = "render")]
    source: Bytes,
}

impl Pdf {
    /// Default bound on a single page's decompressed content, guarding against
    /// a decompression bomb. Used by [`open`](Pdf::open); override it with
    /// [`open_with_limit`](Pdf::open_with_limit).
    pub const DEFAULT_MAX_PAGE_BYTES: NonZeroUsize =
        NonZeroUsize::new(64 * 1024 * 1024).expect("non-zero");

    /// Open a PDF from its bytes, using the [default page
    /// bound](Pdf::DEFAULT_MAX_PAGE_BYTES).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if the
    /// bytes are not a readable PDF.
    pub fn open(document: &[u8]) -> Result<Self> {
        Self::open_with_limit(document, Self::DEFAULT_MAX_PAGE_BYTES)
    }

    /// Open a PDF with an explicit per-page decompressed-size bound.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidDocument`](crate::ErrorKind::InvalidDocument) if the
    /// bytes are not a readable PDF.
    pub fn open_with_limit(document: &[u8], max_page_bytes: NonZeroUsize) -> Result<Self> {
        let doc = Document::load_mem(document)
            .map_err(|e| Error::invalid_document(format!("not a readable PDF: {e}")))?;
        Ok(Self {
            doc,
            max_page_bytes,
            #[cfg(feature = "render")]
            source: Bytes::copy_from_slice(document),
        })
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
                    embeddings.push(Embedding {
                        id,
                        page,
                        kind: EmbeddingKind::from_filters(image.filters.as_deref()),
                        width: image.width.max(0) as u32,
                        height: image.height.max(0) as u32,
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

    /// Rewrite the born-digital text layer with `replacements` and return the
    /// new document bytes.
    ///
    /// **Fail-closed:** if a `find` text cannot be located on its page, the
    /// whole rewrite is refused with
    /// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) rather than
    /// emitting a document where some targeted text survives. An empty
    /// `replacements` re-saves the document unchanged.
    ///
    /// This rewrites the born-digital text layer only; text baked into a page
    /// image has no text layer to rewrite (the `render` feature rasterises such
    /// pages for OCR instead). To also replace embedded images, use
    /// [`rewrite_with_images`](Pdf::rewrite_with_images).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a
    /// replacement could not be applied.
    pub fn rewrite(&self, replacements: &[Replacement]) -> Result<Vec<u8>> {
        self.rewrite_with_images(replacements, &[])
    }

    /// Rewrite the born-digital text layer with `replacements` *and* replace the
    /// stream content of the embedded images named in `images`, returning the
    /// new document bytes.
    ///
    /// **Fail-closed:** a text `find` absent on its page, or an
    /// [`ImageReplacement`] naming an object that is not an image stream in the
    /// document, refuses the whole rewrite rather than emitting a
    /// partially-redacted document.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::UnsafeRewrite`](crate::ErrorKind::UnsafeRewrite) if a text or
    /// image replacement could not be applied.
    pub fn rewrite_with_images(
        &self,
        replacements: &[Replacement],
        images: &[ImageReplacement],
    ) -> Result<Vec<u8>> {
        let mut doc = self.doc.clone();

        for r in replacements {
            // Fail-closed: `replace_text` is a no-op (not an error) when the
            // text is absent, so confirm the target is present first.
            let page_text = doc
                .extract_text(&[r.page])
                .map_err(|e| Error::unsafe_rewrite(format!("page {} unreadable: {e}", r.page)))?;
            if !page_text.contains(r.find.as_str()) {
                return Err(Error::unsafe_rewrite(format!(
                    "text `{}` not found on page {}",
                    r.find, r.page
                )));
            }
            doc.replace_text(r.page, r.find.as_str(), r.replace.as_str(), None)
                .map_err(|e| {
                    Error::unsafe_rewrite(format!(
                        "could not replace `{}` on page {}: {e}",
                        r.find, r.page
                    ))
                })?;
        }

        for image in images {
            let id = image.id.object();
            let object = doc.get_object_mut(id).map_err(|e| {
                Error::unsafe_rewrite(format!(
                    "image ({}, {}) not found: {e}",
                    image.id.number, image.id.generation
                ))
            })?;
            let Object::Stream(stream) = object else {
                return Err(Error::unsafe_rewrite(format!(
                    "object ({}, {}) is not a stream",
                    image.id.number, image.id.generation
                )));
            };
            // Fail-closed: the target must be an image XObject, not some other
            // stream (a page content stream, an embedded font) that happens to
            // share the id — overwriting one of those would corrupt the page.
            let is_image = stream.dict.get(b"Subtype").and_then(Object::as_name).ok()
                == Some(b"Image".as_slice());
            if !is_image {
                return Err(Error::unsafe_rewrite(format!(
                    "object ({}, {}) is not an image XObject",
                    image.id.number, image.id.generation
                )));
            }
            stream.set_content(image.bytes.clone());
        }

        let mut out = Vec::new();
        doc.save_to(&mut out)
            .map_err(|e| Error::invalid_document(format!("could not save PDF: {e}")))?;
        Ok(out)
    }

    /// The original bytes this document was opened from (used by the `render`
    /// feature so PDFium rasterises the pristine PDF, not a lopdf
    /// re-serialisation, which can degrade fidelity on scanned or malformed
    /// documents).
    #[cfg(feature = "render")]
    pub(crate) fn source(&self) -> &Bytes {
        &self.source
    }
}
