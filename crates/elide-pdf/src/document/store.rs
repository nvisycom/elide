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
use elide_core::{Error, Result};
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
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if the input
    /// exceeds [`MAX_DOCUMENT_BYTES`](Self::MAX_DOCUMENT_BYTES), or
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
    /// bytes are not a readable PDF.
    pub(crate) fn open(document: &[u8], max_page_bytes: NonZeroUsize) -> Result<Self> {
        if document.len() > Self::MAX_DOCUMENT_BYTES {
            return Err(Error::resource_limit(format!(
                "document is {} bytes, over the {}-byte limit",
                document.len(),
                Self::MAX_DOCUMENT_BYTES
            )));
        }
        let doc = Document::load_mem(document)
            .map_err(|e| Error::malformed(format!("not a readable PDF: {e}")))?;
        // A document with an empty user password decrypts on load. One that
        // still carries `/Encrypt` needs a password: its strings and streams are
        // ciphertext nothing can extract or redact, so it is refused rather than
        // passed through unread.
        if doc.is_encrypted() {
            return Err(Error::capability_unavailable(
                "PDF is encrypted with a user password, so its content cannot be read",
            ));
        }
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

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;
    use lopdf::{EncryptionState, EncryptionVersion, Object, Permissions, dictionary};

    use super::*;

    /// A one-page PDF encrypted with `user_password`, as bytes.
    fn encrypted(user_password: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages });
        doc.objects.insert(
            pages,
            Object::Dictionary(
                dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 },
            ),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);
        doc.trailer.set(
            "ID",
            vec![
                Object::string_literal("0123456789abcdef"),
                Object::string_literal("0123456789abcdef"),
            ],
        );
        let state = EncryptionState::try_from(EncryptionVersion::V2 {
            document: &doc,
            owner_password: "owner",
            user_password,
            key_length: 128,
            permissions: Permissions::all(),
        })
        .unwrap();
        doc.encrypt(&state).unwrap();
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn a_pdf_that_needs_a_password_is_refused() {
        let error = Store::open(&encrypted("secret"), Store::DEFAULT_MAX_PAGE_BYTES)
            .expect_err("a password-protected PDF");
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
    }

    #[test]
    fn a_pdf_with_an_empty_user_password_opens() {
        let store = Store::open(&encrypted(""), Store::DEFAULT_MAX_PAGE_BYTES).expect("opens");
        assert!(!store.doc().is_encrypted());
    }
}
