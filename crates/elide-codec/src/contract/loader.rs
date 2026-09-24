//! Decoding raw bytes into a typed handle, plus the erasure the registry
//! stores.
//!
//! - [`Loader`]: per-modality decoder a format implementation writes. Returns a
//!   concrete handler implementing [`Handler`](super::Handler).
//! - [`ErasedLoader`]: modality-erased loader the `FormatRegistry` holds behind
//!   `Arc`; every [`Loader`] is one through a blanket impl.

use elide_core::Result;
use elide_core::modality::Modality;

use super::Handler;
use super::document::{DocumentHandle, UntypedDocumentHandle};
use crate::content::ContentData;

/// Per-modality format loader.
///
/// A loader validates and parses raw content for its [`Modality`], producing a
/// handler that implements [`Handler`](super::Handler). Loaders are the leaves
/// the `FormatRegistry` composes: registering a format means registering its
/// loader. A loader serves exactly one modality — its [`Modality`] associated
/// type — so the registry recovers that modality from the loader type alone.
///
/// # Implementing a third-party format
///
/// 1. Implement [`Handler`](super::Handler) for the per-format handler type
///    that owns the parsed in-memory representation.
/// 2. Implement `Loader` for a stateless type whose [`decode`] validates raw
///    [`ContentData`] and returns the handler.
/// 3. Build a [`Format`] with [`Format::new`], chain extensions / content types
///    as needed, and register it on a `FormatRegistry`.
///
/// The registry erases the modality internally; third-party callers never touch
/// the object-safe surface.
///
/// [`Modality`]: Self::Modality
/// [`decode`]: Loader::decode
/// [`Format`]: super::Format
/// [`Format::new`]: super::Format::new
#[async_trait::async_trait]
pub trait Loader: Send + Sync + 'static {
    /// The modality this loader decodes into.
    type Modality: Modality;

    /// The handler type this loader produces.
    type Handler: Handler<Self::Modality>;

    /// Validate and parse the content, returning the loaded handler.
    ///
    /// # Errors
    ///
    /// Returns an error when the content is malformed for this format.
    async fn decode(&self, content: ContentData) -> Result<Self::Handler>;
}

/// Modality-erased loader the `FormatRegistry` holds behind `Arc`.
/// Adapts a [`Loader`] into a uniform `decode` returning an
/// [`UntypedDocumentHandle`].
///
/// Crate-internal: every consumer goes through [`Format::decode`] or
/// `FormatRegistry::decode` instead. Every [`Loader`] is an `ErasedLoader`
/// through the blanket impl below.
///
/// [`Format::decode`]: super::Format::decode
#[async_trait::async_trait]
pub(crate) trait ErasedLoader: Send + Sync + 'static {
    async fn decode(&self, content: ContentData) -> Result<UntypedDocumentHandle>;
}

#[async_trait::async_trait]
impl<L: Loader> ErasedLoader for L {
    async fn decode(&self, content: ContentData) -> Result<UntypedDocumentHandle> {
        let handler = Loader::decode(self, content).await?;
        let format_id = Handler::format(&handler);
        let boxed: Box<dyn Handler<L::Modality>> = Box::new(handler);
        let handle = DocumentHandle::<L::Modality>::new(format_id, boxed);
        Ok(UntypedDocumentHandle::new(handle))
    }
}
