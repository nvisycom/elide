//! [`Document`]: one named, decoded document the orchestrator redacts, plus the
//! traits that shape how it is passed and built.
//!
//! - [`AsDocuments`] lets the orchestrator's methods accept a single
//!   `&mut Document` or a `&mut [Document]` interchangeably.
//! - [`RegistryDocumentExt`] decodes raw bytes straight into a named `Document`.
//! - [`DocumentsExt`] resolves a [`PartId`] path against a slice of documents
//!   (the container the orchestrator's flatten and fold descend from).

use std::path::Path;

use elide_codec::content::ContentData;
use elide_codec::{FormatRegistry, UntypedDocumentHandle};
use elide_core::{Error, ErrorKind, Result};

use crate::PartId;

/// One document the engine redacts: its name, the first segment of every
/// [`PartId`] beneath it, and its decoded handle.
///
/// A report describes a slice of these, analyzed and redacted as one logical
/// unit ([`analyze`] / [`anonymize_with`]); a single document is a one-element
/// slice. The name keys the document's own content (a depth-1 part) and prefixes
/// its parts' paths, so two documents that share a local part id (two scans,
/// each `page-1.png`) stay distinct, the collision a flat id would hit.
///
/// The name is the engine's, not the codec's: [`UntypedDocumentHandle`] is bytes
/// and format, never a filename, so identity is attached here, one layer up.
///
/// [`analyze`]: crate::Orchestrator::analyze
/// [`anonymize_with`]: crate::Orchestrator::anonymize_with
pub struct Document {
    /// The document's name, the first path segment of every part beneath it,
    /// and the key of its own content in the report. Must be unique within the
    /// slice.
    pub name: String,
    /// The document's decoded handle. Redacted in place, ready for its own
    /// `encode`.
    pub handle: UntypedDocumentHandle,
}

impl Document {
    /// A document from a name and an already-decoded handle.
    ///
    /// For the common case of decoding raw bytes into a document in one step,
    /// prefer [`FormatRegistry::document`] / [`FormatRegistry::document_with`]
    /// (the [`RegistryDocumentExt`] methods), which decode and name together.
    ///
    /// [`FormatRegistry::document`]: RegistryDocumentExt::document
    /// [`FormatRegistry::document_with`]: RegistryDocumentExt::document_with
    pub fn new(name: impl Into<String>, handle: UntypedDocumentHandle) -> Self {
        Self {
            name: name.into(),
            handle,
        }
    }
}

/// Seals [`AsDocuments`] and [`RegistryDocumentExt`] so neither is a public
/// extension point, the engine owns their whole impl surface.
mod sealed {
    pub trait Sealed {}
}

/// One or many [`Document`]s, so [`analyze`], [`re_analyze`], [`anonymize_with`],
/// and [`anonymize`] accept a single `&mut Document` or a `&mut [Document]`
/// interchangeably: a single document is a one-element slice.
///
/// Sealed, implemented only for [`Document`] and `[Document]`.
///
/// [`analyze`]: crate::Orchestrator::analyze
/// [`re_analyze`]: crate::Orchestrator::re_analyze
/// [`anonymize_with`]: crate::Orchestrator::anonymize_with
/// [`anonymize`]: crate::Orchestrator::anonymize
pub trait AsDocuments: sealed::Sealed {
    /// The documents as a mutable slice, one element for a single document,
    /// the slice itself for many.
    fn as_documents_mut(&mut self) -> &mut [Document];
}

impl sealed::Sealed for Document {}
impl sealed::Sealed for [Document] {}
impl<const N: usize> sealed::Sealed for [Document; N] {}
impl<T: sealed::Sealed + ?Sized> sealed::Sealed for &mut T {}

impl AsDocuments for Document {
    fn as_documents_mut(&mut self) -> &mut [Document] {
        std::slice::from_mut(self)
    }
}

impl AsDocuments for [Document] {
    fn as_documents_mut(&mut self) -> &mut [Document] {
        self
    }
}

impl<const N: usize> AsDocuments for [Document; N] {
    fn as_documents_mut(&mut self) -> &mut [Document] {
        self
    }
}

/// A `&mut` to anything that is [`AsDocuments`] is too, so a caller holding a
/// `&mut Document` or `&mut [Document]` (as the two-phase [`anonymize`] does
/// internally) passes it straight through.
///
/// [`anonymize`]: crate::Orchestrator::anonymize
impl<T: AsDocuments + ?Sized> AsDocuments for &mut T {
    fn as_documents_mut(&mut self) -> &mut [Document] {
        (**self).as_documents_mut()
    }
}

/// Resolve a [`PartId`] path against a slice of documents: the container tree the
/// orchestrator's flatten and fold descend from. Implemented only for
/// `[Document]`, the shape those passes hold.
pub(crate) trait DocumentsExt {
    /// The document a path descent starts from, and the segments still to walk
    /// within it: the leading segment selects the document, the remaining
    /// segments walk it. `None` when no document matches the leading segment, or
    /// the path is empty.
    ///
    /// Used by [`decode_by_path`](crate::Orchestrator::decode_by_path) so a path
    /// resolves its starting container the same way everywhere.
    fn root_container<'d, 'seg>(
        &'d mut self,
        segments: &'seg [&str],
    ) -> Option<(&'d mut UntypedDocumentHandle, &'seg [&'seg str])>;

    /// The *top* container named by `parent`, if `parent` is a top-level path, a
    /// one-segment document path. `None` for a deeper parent (a nested container
    /// to re-decode). The fold writes straight into a top container, which
    /// re-encodes itself.
    fn top_container(&mut self, parent: &PartId) -> Option<&mut UntypedDocumentHandle>;
}

impl DocumentsExt for [Document] {
    fn root_container<'d, 'seg>(
        &'d mut self,
        segments: &'seg [&str],
    ) -> Option<(&'d mut UntypedDocumentHandle, &'seg [&'seg str])> {
        let (name, rest) = segments.split_first()?;
        let document = self.iter_mut().find(|d| d.name == *name)?;
        Some((&mut document.handle, rest))
    }

    fn top_container(&mut self, parent: &PartId) -> Option<&mut UntypedDocumentHandle> {
        let mut segments = parent.segments();
        let name = segments.next()?;
        // A top document is exactly a one-segment path; anything deeper is a
        // nested container, not a root.
        if segments.next().is_some() {
            return None;
        }
        self.iter_mut()
            .find(|d| d.name == name)
            .map(|d| &mut d.handle)
    }
}

/// Decode raw bytes straight into a named [`Document`], an extension trait on
/// [`FormatRegistry`], so the codec stays byte-and-format only (a handle carries
/// no filename) while the engine attaches the name it owns.
///
/// [`document`] infers the format from the name's own extension (a real filename
/// like `report.docx`); [`document_with`] takes the format explicitly, for a name
/// that carries none or a misleading one.
///
/// Sealed, implemented only for [`FormatRegistry`].
///
/// [`document`]: Self::document
/// [`document_with`]: Self::document_with
pub trait RegistryDocumentExt: sealed::Sealed {
    /// Decode `bytes` into a [`Document`] named `name`, resolving the format from
    /// the extension of `name` itself (`report.docx` → `docx`).
    ///
    /// # Errors
    ///
    /// Returns [`MalformedInput`] when `name` carries no extension to resolve a
    /// format from, use [`document_with`] with an explicit one. Otherwise
    /// propagates the decode error (e.g. [`CapabilityUnavailable`] for an
    /// unregistered format).
    ///
    /// [`document_with`]: Self::document_with
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    /// [`CapabilityUnavailable`]: elide_core::ErrorKind::CapabilityUnavailable
    fn document(
        &self,
        name: impl Into<String>,
        bytes: impl Into<ContentData>,
    ) -> impl std::future::Future<Output = Result<Document>>;

    /// Decode `bytes` into a [`Document`] named `name`, resolving the format from
    /// the explicit `extension` (which always wins, whatever `name` looks like).
    ///
    /// # Errors
    ///
    /// Propagates the decode error (e.g. [`CapabilityUnavailable`] when no format
    /// is registered for `extension`).
    ///
    /// [`CapabilityUnavailable`]: elide_core::ErrorKind::CapabilityUnavailable
    fn document_with(
        &self,
        name: impl Into<String>,
        extension: &str,
        bytes: impl Into<ContentData>,
    ) -> impl std::future::Future<Output = Result<Document>>;
}

impl sealed::Sealed for FormatRegistry {}

impl RegistryDocumentExt for FormatRegistry {
    async fn document(
        &self,
        name: impl Into<String>,
        bytes: impl Into<ContentData>,
    ) -> Result<Document> {
        let name = name.into();
        // The format is the name's own extension, lowercased, resolved via
        // `Path::extension` (which ignores a leading-dot dotfile like `.rels`).
        let extension = Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let Some(extension) = extension else {
            return Err(Error::new(
                ErrorKind::MalformedInput,
                format!(
                    "document name `{name}` has no extension to resolve a format from; \
                     use `document_with` with an explicit extension"
                ),
            ));
        };
        let handle = self.decode(bytes, &extension).await?;
        Ok(Document::new(name, handle))
    }

    async fn document_with(
        &self,
        name: impl Into<String>,
        extension: &str,
        bytes: impl Into<ContentData>,
    ) -> Result<Document> {
        let handle = self.decode(bytes, extension).await?;
        Ok(Document::new(name, handle))
    }
}
