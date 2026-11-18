//! The PDF access layer: an opened document ([`Pdf`]), read through
//! [`reader`], changed through an [`Edit`] of it, and rendered through
//! `render` (feature `render`).
//!
//! Every other module reads a document as a [`Read`] (`pdf.read(Text)`) and
//! changes one as a [`Redact`] (`edit.redact(ImageRedaction { .. })`), so
//! `Pdf` and `Edit` stay the same however many modalities build on them.

pub(crate) mod reader;
#[cfg(feature = "render")]
pub(crate) mod render;
pub(crate) mod writer;

use bytes::Bytes;
use elide_core::{Error, Result};
use lopdf::content::Content;
use lopdf::xref::XrefEntry;
use lopdf::{Document, Object};

use self::reader::{DictionaryExt, DocumentExt, ObjectExt};
pub(crate) use self::writer::Edit;

/// A read of a [`Pdf`]: what a modality takes from a document.
pub(crate) trait Read {
    /// What the read yields.
    type Output;

    /// Read `pdf`.
    ///
    /// # Errors
    ///
    /// As each read documents.
    fn read(self, pdf: &Pdf) -> Result<Self::Output>;
}

/// A redaction written through an [`Edit`]: what a modality changes in a
/// document.
pub(crate) trait Redact {
    /// Apply the redaction to `edit`.
    ///
    /// # Errors
    ///
    /// As each redaction documents.
    fn redact(self, edit: &mut Edit) -> Result<()>;
}

/// An opened PDF: the parsed object graph and the bytes it was parsed from or
/// saved as, kept together so neither is produced twice.
#[derive(Debug, Clone)]
pub(crate) struct Pdf {
    /// The parsed object graph.
    doc: Document,
    /// The bytes `doc` was parsed from or saved as: what PDFium renders, and
    /// what ships when nothing is redacted.
    bytes: Bytes,
}

impl Pdf {
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
        let mut doc = Document::load_mem(document)
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
        if own_file_filter(&doc, document) {
            return Err(Error::capability_unavailable(
                "PDF encrypts its attached files under a crypt filter of their own, \
                 which cannot be read",
            ));
        }
        inline_encodings(&mut doc);
        Ok(Self {
            doc,
            bytes: Bytes::copy_from_slice(document),
        })
    }

    /// The parsed object graph.
    pub(crate) fn doc(&self) -> &Document {
        &self.doc
    }

    /// The document `doc` as saved to `bytes`.
    fn from_parts(doc: Document, bytes: Bytes) -> Self {
        Self { doc, bytes }
    }

    /// The bytes the document was parsed from or saved as.
    pub(crate) fn bytes(&self) -> Bytes {
        self.bytes.clone()
    }

    /// What `read` reads from the document.
    ///
    /// # Errors
    ///
    /// As `read`.
    pub(crate) fn read<R: Read>(&self, read: R) -> Result<R::Output> {
        read.read(self)
    }

    /// An edit of a copy of the document, leaving this one as it is.
    pub(crate) fn edit(&self) -> Edit {
        Edit::of(self.doc.clone())
    }
}

/// Each stream's `/Filter` and `/DecodeParms` with the references in them
/// followed. lopdf reads both only as direct values: it decodes a stream
/// whose filter is a reference as if it had none, so its encoded bytes would
/// be read, and written back, as its content.
fn inline_encodings(doc: &mut Document) {
    let mut inlined = Vec::new();
    for (&id, object) in &doc.objects {
        let Object::Stream(stream) = object else {
            continue;
        };
        for key in [&b"Filter"[..], b"DecodeParms"] {
            if let Ok(value) = stream.dict.get(key) {
                let value = inline(doc, value, 0);
                if stream.dict.get(key).ok() != Some(&value) {
                    inlined.push((id, key, value));
                }
            }
        }
    }
    for (id, key, value) in inlined {
        if let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) {
            stream.dict.set(key, value);
        }
    }
}

/// `value` with the references in it, and in the arrays and dictionaries it
/// holds two levels down, followed; a reference to nothing is null.
fn inline(doc: &Document, value: &Object, depth: usize) -> Object {
    let Some(value) = doc.resolve(value) else {
        return Object::Null;
    };
    match value {
        Object::Array(items) if depth < 2 => Object::Array(
            items
                .iter()
                .map(|item| inline(doc, item, depth + 1))
                .collect(),
        ),
        Object::Dictionary(dict) if depth < 2 => Object::Dictionary(
            dict.iter()
                .map(|(key, value)| (key.clone(), inline(doc, value, depth + 1)))
                .collect(),
        ),
        value => value.clone(),
    }
}

/// Whether the decrypted `doc`, parsed from `bytes`, holds embedded files
/// encrypted under a crypt filter of their own (`/EFF`, 7.6.2), which lopdf
/// does not apply, so their data is still ciphertext: it has an embedded
/// file, and its encryption dictionary selects for them a filter other than
/// its stream default.
///
/// lopdf drops the encryption dictionary on load, so `/EFF` is read from the
/// dictionary's own bytes. When that can't be read, a document that defines
/// a filter other than its defaults is refused, whether or not it selects it.
fn own_file_filter(doc: &Document, bytes: &[u8]) -> bool {
    let Some(state) = &doc.encryption_state else {
        return false;
    };
    let has_files = doc
        .objects
        .values()
        .filter_map(ObjectExt::dict)
        .any(|dict| dict.get(b"EF").is_ok() || dict.is_type(doc, b"EmbeddedFile"));
    if !has_files {
        return false;
    }
    match file_filter(doc, bytes) {
        Some(Some(filter)) => filter != state.default_stream_filter(),
        Some(None) => false,
        None => {
            let defaults = [state.default_stream_filter(), state.default_string_filter()];
            state
                .crypt_filters()
                .keys()
                .any(|name| !defaults.contains(&name.as_slice()))
        }
    }
}

/// The `/EFF` of `doc`'s encryption dictionary, read from `bytes` with
/// lopdf's content parser (which reads a dictionary as an operand):
/// `Some(None)` when it has none, `None` when the dictionary, or an `/EFF`
/// that is not a name, cannot be read.
fn file_filter(doc: &Document, bytes: &[u8]) -> Option<Option<Vec<u8>>> {
    let id = doc.encryption_state.as_ref()?.encrypt_object_id()?;
    let Some(XrefEntry::Normal { offset, .. }) = doc.reference_table.get(id.0) else {
        return None;
    };
    let object = bytes.get(usize::try_from(*offset).ok()?..)?;
    let end = object
        .windows(b"endobj".len())
        .position(|window| window == b"endobj")?;
    let content = Content::decode(&object[..end + b"endobj".len()]).ok()?;
    let dict = content
        .operations
        .iter()
        .find(|operation| operation.operator == "endobj")?
        .operands
        .first()?
        .as_dict()
        .ok()?;
    match dict.get(b"EFF") {
        Err(_) => Some(None),
        Ok(Object::Name(name)) => Some(Some(name.clone())),
        Ok(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use elide_core::ErrorKind;
    use lopdf::encryption::crypt_filters::{Aes128CryptFilter, CryptFilter};
    use lopdf::{
        Dictionary, EncryptionState, EncryptionVersion, Object, Permissions, Stream, dictionary,
    };

    use super::*;
    use crate::fixtures::with_pages;

    /// A one-page PDF, unencrypted.
    fn document() -> Document {
        let mut doc = Document::with_version("1.7");
        with_pages(&mut doc, [Dictionary::new()]);
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
    fn saved(doc: Document, version: EncryptionVersion<'_>) -> Vec<u8> {
        saved_selecting(doc, version, None)
    }

    /// `doc` encrypted under `version`, its encryption dictionary selecting
    /// `files` for attached files when given, as bytes.
    fn saved_selecting(
        mut doc: Document,
        version: EncryptionVersion<'_>,
        files: Option<&str>,
    ) -> Vec<u8> {
        let state = EncryptionState::try_from(version).unwrap();
        doc.encrypt(&state).unwrap();
        if let Some(files) = files {
            let id = doc.trailer.get(b"Encrypt").unwrap().as_reference().unwrap();
            doc.get_dictionary_mut(id).unwrap().set("EFF", files);
        }
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
        let error = Pdf::open(&encrypted("secret")).expect_err("a password-protected PDF");
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
    }

    #[test]
    fn a_pdf_with_an_empty_user_password_opens() {
        let pdf = Pdf::open(&encrypted("")).expect("opens");
        assert!(!pdf.doc().is_encrypted());
    }

    /// Attached files under a crypt filter of their own are still ciphertext
    /// once lopdf decrypts the rest, so the document is refused; one that
    /// only defines such a filter, or selects the stream default for them,
    /// is read.
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
        let with = |names: &[&[u8]], files: Option<&str>| {
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
            saved_selecting(doc.clone(), version, files)
        };
        Pdf::open(&with(&[b"StdCF"], None)).expect("files under the stream default open");
        Pdf::open(&with(&[b"StdCF", b"FileCF"], None)).expect("a filter defined, not selected");
        Pdf::open(&with(&[b"StdCF", b"FileCF"], Some("StdCF"))).expect("the default selected");
        let error =
            Pdf::open(&with(&[b"StdCF", b"FileCF"], Some("FileCF"))).expect_err("their own filter");
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
        assert!(error.to_string().contains("attached files"), "{error}");
    }
}
