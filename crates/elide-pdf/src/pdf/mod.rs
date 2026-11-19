//! The PDF access layer: an opened document ([`Pdf`]), read through
//! [`reader`] over hayro-syntax, changed through an [`Edit`] of it, written
//! through [`writer`] over pdf-writer, and rendered through `render`
//! (feature `render`).
//!
//! Every other module reads a document as a [`Read`] (`pdf.read(Text)`) and
//! changes one as a [`Redact`] (`edit.redact(ImageRedaction { .. })`), so
//! `Pdf` and `Edit` stay the same however many modalities build on them.

pub(crate) mod content;
pub(crate) mod decode;
pub(crate) mod object;
pub(crate) mod reader;
#[cfg(feature = "render")]
pub(crate) mod render;
pub(crate) mod writer;

use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use elide_core::{Error, Result};
use hayro_syntax::object::{Dict, MaybeRef, Object};
use hayro_syntax::{DecryptionError, LoadPdfError};

use self::reader::ObjectId;
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

/// An opened PDF: the document read, and the bytes it was read from.
#[derive(Clone)]
pub(crate) struct Pdf {
    file: Arc<hayro_syntax::Pdf>,
    /// The bytes the document was read from: what PDFium renders, and what
    /// ships when nothing is redacted.
    bytes: Bytes,
}

impl fmt::Debug for Pdf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pdf")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl Pdf {
    /// Maximum accepted source-document size (64 MiB).
    pub(crate) const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

    /// Read `document`.
    ///
    /// # Errors
    ///
    /// As [`open_bytes`](Self::open_bytes).
    pub(crate) fn open(document: &[u8]) -> Result<Self> {
        Self::open_bytes(Bytes::copy_from_slice(document))
    }

    /// Read `document`, keeping its bytes.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::ResourceLimit`](elide_core::ErrorKind::ResourceLimit) if
    /// the input exceeds [`MAX_DOCUMENT_BYTES`](Self::MAX_DOCUMENT_BYTES);
    /// [`ErrorKind::CapabilityUnavailable`](elide_core::ErrorKind::CapabilityUnavailable)
    /// if it is encrypted and does not open without a password, or with an
    /// encryption that is not read; or
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the bytes are not a readable PDF.
    pub(crate) fn open_bytes(document: Bytes) -> Result<Self> {
        if document.len() > Self::MAX_DOCUMENT_BYTES {
            return Err(Error::resource_limit(format!(
                "document is {} bytes, over the {}-byte limit",
                document.len(),
                Self::MAX_DOCUMENT_BYTES
            )));
        }
        let file = hayro_syntax::Pdf::new(Arc::new(document.clone())).map_err(|e| match e {
            LoadPdfError::Decryption(DecryptionError::PasswordProtected) => {
                Error::capability_unavailable(
                    "PDF is encrypted and could not be decrypted without a password, so its \
                     content cannot be read",
                )
            }
            LoadPdfError::Decryption(e) => Error::capability_unavailable(format!(
                "PDF is encrypted in a way that cannot be read ({e:?})"
            )),
            LoadPdfError::Invalid => Error::malformed("not a readable PDF"),
        })?;
        Ok(Self {
            file: Arc::new(file),
            bytes: document,
        })
    }

    /// The document read.
    pub(crate) fn file(&self) -> &hayro_syntax::Pdf {
        &self.file
    }

    /// The bytes the document was read from.
    pub(crate) fn bytes(&self) -> Bytes {
        self.bytes.clone()
    }

    /// The object `id`.
    pub(crate) fn object(&self, id: ObjectId) -> Option<Object<'_>> {
        self.file.xref().get::<Object<'_>>(id)
    }

    /// What `item` holds: the value in place, or the object it refers to;
    /// `None` when it refers to nothing.
    pub(crate) fn resolve<'a>(&'a self, item: &MaybeRef<Object<'a>>) -> Option<Object<'a>> {
        match item {
            MaybeRef::NotRef(object) => Some(object.clone()),
            MaybeRef::Ref(reference) => self.object((*reference).into()),
        }
    }

    /// The catalog.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::MalformedInput`](elide_core::ErrorKind::MalformedInput) if
    /// the trailer's `/Root` is not a dictionary.
    pub(crate) fn catalog(&self) -> Result<Dict<'_>> {
        let root = self.file.xref().root_id();
        self.file
            .xref()
            .get::<Dict<'_>>(root)
            .ok_or_else(|| Error::malformed("PDF has no catalog"))
    }

    /// The page dictionaries, in order.
    pub(crate) fn pages(&self) -> Vec<Dict<'_>> {
        self.file
            .pages()
            .iter()
            .map(|page| page.raw().clone())
            .collect()
    }

    /// What `read` reads from the document.
    ///
    /// # Errors
    ///
    /// As `read`.
    pub(crate) fn read<R: Read>(&self, read: R) -> Result<R::Output> {
        read.read(self)
    }

    /// An edit of the document, leaving this one as it is.
    pub(crate) fn edit(&self) -> Edit {
        Edit::of(self)
    }
}

#[cfg(test)]
mod tests {
    use elide_core::ErrorKind;

    use super::*;
    use crate::pdf::object::{Stream, dict};
    use crate::pdf::writer::Document;

    /// A document that saves past the limit it would be opened within is
    /// refused when written, not when opened again.
    #[test]
    fn a_document_saved_past_the_limit_is_refused() {
        let mut doc = Document::new((1, 7));
        let large = doc.add(Stream::new(dict! {}, vec![0; Pdf::MAX_DOCUMENT_BYTES]));
        let pages = doc.add(dict! { "Type" => "Pages", "Kids" => Vec::new(), "Count" => 0 });
        let catalog = doc.add(dict! { "Type" => "Catalog", "Pages" => pages, "Large" => large });
        doc.set_root(catalog);
        let error = writer::finish(&doc).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    /// One page, RC4-encrypted under the user password `secret`.
    const NEEDS_PASSWORD: &[u8] = include_bytes!("../../testdata/needs-password.pdf");

    /// One page, RC4-encrypted under an empty user password.
    const EMPTY_PASSWORD: &[u8] = include_bytes!("../../testdata/empty-password.pdf");

    #[test]
    fn a_pdf_that_needs_a_password_is_refused() {
        let error = Pdf::open(NEEDS_PASSWORD).expect_err("a password-protected PDF");
        assert_eq!(error.kind(), ErrorKind::CapabilityUnavailable);
    }

    #[test]
    fn a_pdf_with_an_empty_user_password_opens() {
        let pdf = Pdf::open(EMPTY_PASSWORD).expect("opens");
        assert_eq!(pdf.pages().len(), 1);
    }
}
