//! Format identity: what kind of thing a registered codec is.
//!
//! - [`FormatId`]: stable string identifier (e.g. `"elide.text.txt"`).
//!   Open namespace, no central enum.
//! - [`Format`]: descriptor the `FormatRegistry` indexes by id /
//!   extension / content type. Bundles a [`FormatId`], the modality name
//!   it produces, lookup keys, and an erased loader that decodes bytes
//!   into a typed handle.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use elide_core::Result;

use super::loader::LeafLoader;
use super::{Document, DocumentLoader, Loader};
use crate::content::ContentData;

/// Stable identifier for a registered codec format.
///
/// Open string namespace: downstream crates ship their own formats by
/// registering a [`Format`] with a unique [`FormatId`].
///
/// Convention: dot-separated namespace. Built-in formats use the
/// `elide.` prefix (e.g. `"elide.text.txt"`). Third-party formats use
/// their own (e.g. `"acme.parquet.v2"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatId(Cow<'static, str>);

impl FormatId {
    /// Construct from a static string literal, with no allocation.
    pub const fn new(id: &'static str) -> Self {
        Self(Cow::Borrowed(id))
    }

    /// Borrow as `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FormatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for FormatId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Descriptor for one registered codec format.
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
    /// Build a leaf [`Format`] from a per-modality [`Loader`]: its one handler
    /// becomes the single stream of a leaf [`Document`] (wrapped in
    /// [`LeafLoader`]).
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

    /// Build a multi-part [`Format`] from a [`DocumentLoader`], which decodes
    /// bytes into a [`Document`] of several parts (a stream plus blob sub-parts)
    /// and its recombiner (e.g. a raster format whose `#exif` sub-part is
    /// metadata).
    pub fn with_document_loader<D: DocumentLoader>(id: FormatId, loader: D) -> Self {
        Self {
            id,
            extensions: Vec::new(),
            content_types: Vec::new(),
            loader: Arc::new(loader),
        }
    }

    /// Declare the file extensions (lowercased, no leading dot) that
    /// resolve to this format. Extends any previously-declared list.
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

    /// Declare the MIME content types (lowercased) that resolve to this
    /// format. Extends any previously-declared list.
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
    pub fn id(&self) -> &FormatId {
        &self.id
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

    /// Decode raw content through this format's loader into a [`Document`].
    /// Equivalent to resolving the format yourself and calling
    /// `FormatRegistry::decode`.
    ///
    /// # Errors
    ///
    /// Propagates the loader's decode error.
    pub async fn decode(&self, content: ContentData) -> Result<Document> {
        self.loader.decode(content).await
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
