//! [`Store`]: the opened-document data holder the capabilities operate over.
//!
//! `Store` owns the parsed object graph and the bytes it was read from or
//! saved as, kept together so neither is produced twice. It carries no
//! extraction/inspection/redaction logic; those capabilities are functions in
//! their own modules that take a `&Store` (or `&mut Store`) and read the object
//! graph through it. The public [`Pdf`](super::Pdf) is a thin orchestrator over
//! one `Store`.

use bytes::Bytes;
use elide_core::{Error, Result};
use lopdf::{Document, Object};

use crate::document::object::ObjectExt;

/// The opened-document data holder: the parsed object graph and its bytes.
#[derive(Debug, Clone)]
pub(crate) struct Store {
    /// The parsed object graph. Capabilities read and clone it through the
    /// crate-visible accessors below.
    doc: Document,
    /// The bytes `doc` was parsed from or saved as: what PDFium renders, and
    /// what ships when nothing is redacted.
    bytes: Bytes,
}

impl Store {
    /// Maximum accepted source-document size (64 MiB).
    pub(crate) const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

    /// Parse `document`, retaining the source bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if the input
    /// exceeds [`MAX_DOCUMENT_BYTES`](Self::MAX_DOCUMENT_BYTES), or
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if the
    /// bytes are not a readable PDF.
    pub(crate) fn open(document: &[u8]) -> Result<Self> {
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
        if own_file_filter(&doc) {
            return Err(Error::capability_unavailable(
                "PDF encrypts its attached files under a crypt filter of their own, \
                 which cannot be read",
            ));
        }
        Ok(Self {
            doc,
            bytes: Bytes::copy_from_slice(document),
        })
    }

    /// The in-memory `doc` saved, keeping both: every object it no longer
    /// reaches is dropped first (a superseded content stream, a replaced
    /// image's mask), since lopdf saves an unreachable object too.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the document cannot be saved.
    pub(crate) fn saved(mut doc: Document) -> Result<Self> {
        doc.objects.retain(|_, object| !object.is_xref_stream());
        doc.prune_objects();
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes)
            .map_err(|e| Error::malformed(format!("could not save PDF: {e}")))?;
        Ok(Self {
            doc,
            bytes: Bytes::from(bytes),
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

    /// The bytes the document was parsed from or saved as.
    pub(crate) fn bytes(&self) -> Bytes {
        self.bytes.clone()
    }
}

/// Whether the decrypted `doc` may hold embedded files encrypted under a
/// crypt filter of their own (`/EFF`, 7.6.2), which lopdf does not apply, so
/// their data is still ciphertext: it defines a crypt filter that is neither
/// its stream nor its string default, and has an embedded file.
///
/// lopdf drops the encryption dictionary on load and keeps no `/EFF`, so the
/// filter the files use cannot be read: a document that defines such a filter
/// without selecting it is refused too.
fn own_file_filter(doc: &Document) -> bool {
    let Some(state) = &doc.encryption_state else {
        return false;
    };
    let defaults = [state.default_stream_filter(), state.default_string_filter()];
    let other = state
        .crypt_filters()
        .keys()
        .any(|name| !defaults.contains(&name.as_slice()));
    other
        && doc.objects.values().any(|object| {
            let dict = match object {
                Object::Dictionary(dict) => dict,
                Object::Stream(stream) => &stream.dict,
                _ => return false,
            };
            dict.get(b"EF").is_ok() || dict.has_type(b"EmbeddedFile")
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use elide_core::ErrorKind;
    use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
    use lopdf::{EncryptionState, EncryptionVersion, Permissions, Stream, dictionary};

    use super::*;

    /// A one-page PDF, unencrypted.
    fn document() -> Document {
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
        doc
    }

    /// `doc` encrypted under `version`, as bytes.
    fn saved(mut doc: Document, version: EncryptionVersion<'_>) -> Vec<u8> {
        let state = EncryptionState::try_from(version).unwrap();
        doc.encrypt(&state).unwrap();
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    /// A one-page PDF encrypted with `user_password`, as bytes.
    fn encrypted(user_password: &str) -> Vec<u8> {
        let doc = document();
        let version = EncryptionVersion::V2 {
            document: &doc,
            owner_password: "owner",
            user_password,
            key_length: 128,
            permissions: Permissions::all(),
        };
        saved(doc.clone(), version)
    }

    #[test]
    fn a_pdf_that_needs_a_password_is_refused() {
        let error = Store::open(&encrypted("secret")).expect_err("a password-protected PDF");
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
    }

    #[test]
    fn a_pdf_with_an_empty_user_password_opens() {
        let store = Store::open(&encrypted("")).expect("opens");
        assert!(!store.doc().is_encrypted());
    }

    /// Attached files under a crypt filter of their own are still ciphertext
    /// once lopdf decrypts the rest, so the document is refused.
    #[test]
    fn a_pdf_whose_files_have_their_own_crypt_filter_is_refused() {
        let mut doc = document();
        let data = doc.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile" },
            b"Jane Roe".to_vec(),
        ));
        doc.add_object(dictionary! {
            "Type" => "Filespec", "F" => Object::string_literal("a.txt"),
            "EF" => dictionary! { "F" => data },
        });
        let aes: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
        let with = |names: &[&[u8]]| {
            let filters = names
                .iter()
                .map(|name| (name.to_vec(), aes.clone()))
                .collect::<BTreeMap<_, _>>();
            let version = EncryptionVersion::V4 {
                document: &doc,
                encrypt_metadata: true,
                crypt_filters: filters,
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
                owner_password: "owner",
                user_password: "",
                permissions: Permissions::all(),
            };
            saved(doc.clone(), version)
        };
        Store::open(&with(&[b"StdCF"])).expect("files under the stream default open");
        let error = Store::open(&with(&[b"StdCF", b"FileCF"])).expect_err("their own filter");
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
        assert!(error.to_string().contains("attached files"), "{error}");
    }
}
