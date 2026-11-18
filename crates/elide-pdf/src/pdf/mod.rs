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
use lopdf::{Dictionary, Document, Object};

use self::reader::{DictionaryExt, DocumentExt};
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
        // A document with an empty user password decrypts on load, and its
        // trailer loses `/Encrypt`. One that still carries it was not
        // decrypted (it needs a password, or its encryption dictionary could
        // not be read): its strings and streams are ciphertext nothing can
        // extract or redact, so it is refused rather than passed through
        // unread.
        if doc.trailer.has(b"Encrypt") {
            return Err(Error::capability_unavailable(
                "PDF is encrypted and could not be decrypted without a password, so its \
                 content cannot be read",
            ));
        }
        if own_file_filter(&doc, document) {
            return Err(Error::capability_unavailable(
                "PDF encrypts its attached files under a crypt filter of their own, \
                 which cannot be read",
            ));
        }
        inline_encodings(&mut doc)?;
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
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
/// the values inlined, shared ones counted once per stream that holds them,
/// come to more than [`MAX_INLINED_BYTES`].
fn inline_encodings(doc: &mut Document) -> Result<()> {
    let mut budget = MAX_INLINED_BYTES;
    let mut inlined = Vec::new();
    for (&id, object) in &doc.objects {
        let Object::Stream(stream) = object else {
            continue;
        };
        for key in [&b"Filter"[..], b"DecodeParms"] {
            if let Ok(value) = stream.dict.get(key)
                && holds_reference(value, 0)
            {
                inlined.push((id, key, inline(doc, value, 0, &mut budget)?));
            }
        }
    }
    for (id, key, value) in inlined {
        if let Some(Object::Stream(stream)) = doc.objects.get_mut(&id) {
            stream.dict.set(key, value);
        }
    }
    Ok(())
}

/// The most bytes of values [`inline_encodings`] copies into a document's
/// stream dictionaries, each value costing [`INLINED_VALUE_COST`] and a
/// string or name its bytes too.
const MAX_INLINED_BYTES: usize = 16 * 1024 * 1024;

/// What one inlined value costs, beyond its bytes.
const INLINED_VALUE_COST: usize = 16;

/// Whether `value`, or what it holds two levels down, is a reference.
fn holds_reference(value: &Object, depth: usize) -> bool {
    match value {
        Object::Reference(_) => true,
        Object::Array(items) if depth < 2 => items.iter().any(|v| holds_reference(v, depth + 1)),
        Object::Dictionary(dict) if depth < 2 => {
            dict.iter().any(|(_, v)| holds_reference(v, depth + 1))
        }
        _ => false,
    }
}

/// `value` with the references in it, and in the arrays and dictionaries it
/// holds two levels down, followed; a reference to nothing is null. Each
/// value copied is taken from `budget`.
///
/// # Errors
///
/// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
/// `budget` runs out.
fn inline(doc: &Document, value: &Object, depth: usize, budget: &mut usize) -> Result<Object> {
    let resolved = doc.resolve(value);
    let followed = depth < 2 && matches!(resolved, Some(Object::Array(_) | Object::Dictionary(_)));
    // A value not followed into is copied whole, at its whole size; a null
    // standing for a reference to nothing costs a value too.
    let cost = match resolved {
        Some(value) if !followed => size(value),
        _ => INLINED_VALUE_COST,
    };
    *budget = budget.checked_sub(cost).ok_or_else(|| {
        Error::resource_limit(format!(
            "stream encodings reference more than {MAX_INLINED_BYTES} bytes of values"
        ))
    })?;
    let Some(value) = resolved else {
        return Ok(Object::Null);
    };
    Ok(match value {
        Object::Array(items) if depth < 2 => Object::Array(
            items
                .iter()
                .map(|item| inline(doc, item, depth + 1, budget))
                .collect::<Result<_>>()?,
        ),
        Object::Dictionary(dict) if depth < 2 => Object::Dictionary(
            dict.iter()
                .map(|(key, value)| Ok((key.clone(), inline(doc, value, depth + 1, budget)?)))
                .collect::<Result<_>>()?,
        ),
        value => value.clone(),
    })
}

/// What a copy of `value` costs: [`INLINED_VALUE_COST`] for it and each
/// value it holds, and the bytes of each string and name.
fn size(value: &Object) -> usize {
    INLINED_VALUE_COST
        + match value {
            Object::String(bytes, _) | Object::Name(bytes) => bytes.len(),
            Object::Array(items) => items.iter().map(size).sum(),
            Object::Dictionary(dict) => dict.iter().map(|(key, v)| key.len() + size(v)).sum(),
            Object::Stream(stream) => stream.content.len(),
            _ => 0,
        }
}

/// How many `endobj`s past the encryption dictionary's offset are tried as
/// its end.
const MAX_ENCRYPT_ENDS: usize = 16;

/// Whether the decrypted `doc`, parsed from `bytes`, holds embedded files
/// encrypted under a crypt filter of their own (`/EFF`, 7.6.2), which lopdf
/// does not apply, so their data is still ciphertext: it has an embedded
/// file, and its encryption dictionary selects for them a filter other than
/// its stream default.
///
/// lopdf drops the encryption dictionary on load, so `/EFF` is read from the
/// dictionary's own bytes. When that can't be read, the filter the files are
/// under is unknown (it may even be `Identity`), so the document is refused.
fn own_file_filter(doc: &Document, bytes: &[u8]) -> bool {
    let Some(state) = &doc.encryption_state else {
        return false;
    };
    // A file is found wherever a file specification may be held: as an
    // object, or in place in the attachment tree or an `/AF` array.
    let has_files = doc
        .objects
        .values()
        .any(|object| embeds_files(doc, object, 0));
    if !has_files {
        return false;
    }
    // An omitted `/StmF` is `Identity`, which lopdf holds as no name.
    let streams = match state.default_stream_filter() {
        b"" => &b"Identity"[..],
        name => name,
    };
    match file_filter(doc, bytes) {
        Some(Some(filter)) => filter != streams,
        Some(None) => false,
        None => true,
    }
}

/// Whether `object`, or a value it holds up to [`MAX_ENCRYPT_DEPTH`] levels
/// down, is a file specification embedding a file (its `/EF` names a
/// stream) or an embedded file stream.
fn embeds_files(doc: &Document, object: &Object, depth: usize) -> bool {
    if depth > MAX_ENCRYPT_DEPTH {
        return false;
    }
    let embeds = |dict: &Dictionary| {
        dict.get(b"EF")
            .ok()
            .and_then(|files| doc.resolve_dict(files))
            .is_some_and(|files| {
                files
                    .iter()
                    .any(|(_, file)| matches!(doc.resolve(file), Some(Object::Stream(_))))
            })
    };
    match object {
        Object::Dictionary(dict) => {
            embeds(dict)
                || dict
                    .iter()
                    .any(|(_, value)| embeds_files(doc, value, depth + 1))
        }
        Object::Stream(stream) => embeds(&stream.dict) || stream.dict.is_type(doc, b"EmbeddedFile"),
        Object::Array(items) => items.iter().any(|item| embeds_files(doc, item, depth + 1)),
        _ => false,
    }
}

/// How deep [`embeds_files`] looks into an object's values held in place.
const MAX_ENCRYPT_DEPTH: usize = 32;

/// The `/EFF` of `doc`'s encryption dictionary, read from `bytes` with
/// lopdf's content parser (which reads a dictionary as an operand):
/// `Some(None)` when it has none, `None` when the dictionary, or an `/EFF`
/// that is not a name, cannot be read.
///
/// The dictionary ends at the first `endobj` that parses as the operator
/// closing it: one inside a comment or a string is no end, so each is tried
/// in turn, up to [`MAX_ENCRYPT_ENDS`].
fn file_filter(doc: &Document, bytes: &[u8]) -> Option<Option<Vec<u8>>> {
    let id = doc.encryption_state.as_ref()?.encrypt_object_id()?;
    let Some(XrefEntry::Normal { offset, .. }) = doc.reference_table.get(id.0) else {
        return None;
    };
    let object = bytes.get(usize::try_from(*offset).ok()?..)?;
    let dict = object
        .windows(b"endobj".len())
        .enumerate()
        .filter(|(_, window)| *window == b"endobj")
        .take(MAX_ENCRYPT_ENDS)
        .find_map(|(end, _)| {
            let content = Content::decode(&object[..end + b"endobj".len()]).ok()?;
            let closing = content.operations.into_iter().next_back()?;
            (closing.operator == "endobj").then_some(closing)
        })?
        .operands
        .into_iter()
        .find_map(|operand| operand.as_dict().ok().cloned())?;
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

    /// A document that saves past the limit it would be opened within is
    /// refused when written, not when opened again.
    #[test]
    fn a_document_saved_past_the_limit_is_refused() {
        let mut doc = Document::with_version("1.7");
        with_pages(&mut doc, [Dictionary::new()]);
        let large = doc.add_object(
            Stream::new(dictionary! {}, vec![0; Pdf::MAX_DOCUMENT_BYTES]).with_compression(false),
        );
        doc.trailer.set("Large", large);
        let error = Edit::of(doc).finish().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    /// A large value many streams name as their encoding is inlined into
    /// each, so the copies are bounded together: past the bound the
    /// document is refused, not expanded.
    #[test]
    fn inlined_encodings_are_bounded() {
        let mut doc = Document::with_version("1.7");
        with_pages(&mut doc, [Dictionary::new()]);
        let shared = doc.add_object(dictionary! {
            "Padding" => vec![Object::Integer(0); 100_000],
        });
        for _ in 0..20 {
            doc.add_object(Stream::new(
                dictionary! { "DecodeParms" => shared },
                Vec::new(),
            ));
        }
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let error = Pdf::open(&bytes).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

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

    /// A document whose encryption dictionary lopdf cannot read is loaded
    /// undecrypted, so it is refused rather than read as ciphertext: here an
    /// `endobj` in a name ends the dictionary early for lopdf.
    #[test]
    fn a_pdf_lopdf_could_not_decrypt_is_refused() {
        let doc = document();
        let version = EncryptionVersion::V2 {
            document: &doc,
            owner_password: "owner",
            user_password: "",
            key_length: 128,
            permissions: Permissions::all(),
        };
        let state = EncryptionState::try_from(version).unwrap();
        let mut doc = doc.clone();
        doc.encrypt(&state).unwrap();
        let id = doc.trailer.get(b"Encrypt").unwrap().as_reference().unwrap();
        doc.get_dictionary_mut(id)
            .unwrap()
            .set("Note", Object::Name(b"endobj".to_vec()));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        let error = Pdf::open(&bytes).expect_err("an undecrypted PDF");
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
        // The file's specification is held in place in the attachment tree,
        // and its stream names no type.
        let data = doc.add_object(Stream::new(dictionary! {}, b"Jane Roe".to_vec()));
        doc.catalog_mut().unwrap().set(
            "Names",
            dictionary! {
                "EmbeddedFiles" => dictionary! {
                    "Names" => vec![
                        Object::string_literal("a"),
                        Object::Dictionary(dictionary! {
                            "Type" => "Filespec", "F" => Object::string_literal("a.txt"),
                            "EF" => dictionary! { "F" => data },
                        }),
                    ],
                },
            },
        );
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
