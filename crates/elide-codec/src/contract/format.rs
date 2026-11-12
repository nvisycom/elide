//! Format identity: what kind of thing a registered codec is.
//!
//! - [`FormatId`]: stable string identifier (e.g. `"elide.text.txt"`).
//!   Open namespace, no central enum.
//! - [`Format`]: descriptor the `FormatRegistry` indexes by id /
//!   extension / content type. Bundles a [`FormatId`], its lookup keys
//!   (extensions and content types), and a loader that decodes bytes into
//!   a [`Document`] — an ordered set of parts, each of its own modality,
//!   since one format may produce several (a container spans modalities).

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use elide_core::Result;
use elide_core::redaction::HiddenContentPolicy;

use super::{Document, DocumentLoader, LeafLoader, Loader};
use crate::content::ContentData;

/// A registered codec format's stable identifier, like `elide.text.txt`.
///
/// Open string namespace: downstream crates ship their own formats by
/// registering a [`Format`] with a unique [`FormatId`].
///
/// Convention: dot-separated namespace. Built-in formats use the
/// `elide.` prefix (e.g. `"elide.text.txt"`). Third-party formats use
/// their own (e.g. `"acme.parquet.v2"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FormatId(&'static str);

impl FormatId {
    /// The format id `id`.
    pub const fn new(id: &'static str) -> Self {
        Self(id)
    }

    /// The id as a string slice.
    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for FormatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl AsRef<str> for FormatId {
    fn as_ref(&self) -> &str {
        self.0
    }
}

/// The descriptor of one codec format: its id, lookup keys, and loader.
///
/// Indexed by `FormatRegistry` under its [`FormatId`], every extension
/// in `extensions`, and every MIME in `content_types`.
///
/// Construct via [`Format::new`] (a leaf format from its [`Loader`]) or
/// [`Format::with_document_loader`] (a multi-part format from its
/// [`DocumentLoader`]); read the parts via the accessor methods. The fields
/// are crate-private so a constructor stays the only path that produces a
/// [`Format`], and every format decodes into a [`Document`].
#[derive(Clone)]
pub struct Format {
    pub(crate) id: FormatId,
    pub(crate) extensions: Vec<Cow<'static, str>>,
    pub(crate) content_types: Vec<Cow<'static, str>>,
    pub(crate) loader: Arc<dyn DocumentLoader>,
}

impl Format {
    /// Build a leaf [`Format`] from a per-modality [`Loader`] of one stream.
    ///
    /// The stream it decodes becomes the single part of a leaf [`Document`].
    ///
    /// Extensions and content types default to empty; chain
    /// [`with_extensions`] / [`with_content_types`] to declare the lookup
    /// keys the `FormatRegistry` indexes this format under.
    ///
    /// [`with_extensions`]: Self::with_extensions
    /// [`with_content_types`]: Self::with_content_types
    pub fn new<L: Loader>(id: FormatId, loader: L) -> Self {
        Self {
            id,
            extensions: Vec::new(),
            content_types: Vec::new(),
            loader: Arc::new(LeafLoader(loader)),
        }
    }

    /// Build a [`Format`] from a [`DocumentLoader`] that decodes a whole
    /// document.
    ///
    /// For a format whose [`Document`] has several streams, embedded blobs, or
    /// hidden content to sanitize (e.g. a raster format whose pixels and EXIF
    /// metadata are two streams).
    pub fn with_document_loader<D: DocumentLoader>(id: FormatId, loader: D) -> Self {
        Self {
            id,
            extensions: Vec::new(),
            content_types: Vec::new(),
            loader: Arc::new(loader),
        }
    }

    /// Declare the file extensions that resolve to this format, like `docx`.
    ///
    /// Lowercased, with no leading dot. Extends any previously-declared list.
    #[must_use]
    pub fn with_extensions<I, S>(mut self, extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'static, str>>,
    {
        self.extensions
            .extend(extensions.into_iter().map(Into::into));
        self
    }

    /// Declare the MIME content types that resolve to this format.
    ///
    /// Lowercased. Extends any previously-declared list.
    #[must_use]
    pub fn with_content_types<I, S>(mut self, content_types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Cow<'static, str>>,
    {
        self.content_types
            .extend(content_types.into_iter().map(Into::into));
        self
    }

    /// Stable identifier of this format.
    pub fn id(&self) -> FormatId {
        self.id
    }

    /// File extensions (lowercased, no leading dot) that resolve to this
    /// format.
    pub fn extensions(&self) -> &[Cow<'static, str>] {
        &self.extensions
    }

    /// MIME content types (lowercased) that resolve to this format.
    pub fn content_types(&self) -> &[Cow<'static, str>] {
        &self.content_types
    }

    /// Decode raw content into a [`Document`], stripping hidden content under
    /// `policy`.
    ///
    /// The format's loader decodes the bytes, and `policy` is recorded on the
    /// document so every document embedded in it is decoded under the same one
    /// ([`Document::hidden_content_policy`]). Equivalent to resolving the
    /// format yourself and calling `FormatRegistry::decode`.
    ///
    /// # Errors
    ///
    /// Propagates the loader's decode error.
    pub async fn decode(
        &self,
        content: ContentData,
        policy: HiddenContentPolicy,
    ) -> Result<Document> {
        let document = self.loader.decode(content, policy).await?;
        Ok(document.decoded_under(policy))
    }
}

impl fmt::Debug for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Format")
            .field("id", &self.id)
            .field("extensions", &self.extensions)
            .field("content_types", &self.content_types)
            .finish_non_exhaustive()
    }
}
