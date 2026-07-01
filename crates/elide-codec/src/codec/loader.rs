//! Decoding raw bytes into a typed handle, plus the erasure machinery
//! the registry stores.
//!
//! - [`Loader<M>`]: per-modality decoder a format implementation
//!   writes. Returns a concrete handler implementing [`Handler<M>`].
//! - [`ErasedLoader`]: modality-erased loader the [`FormatRegistry`]
//!   holds behind `Arc`.
//! - [`erase`]: bridge from a typed `Loader<M>` to
//!   `Arc<dyn ErasedLoader>`.
//!
//! [`Handler<M>`]: super::Handler
//! [`FormatRegistry`]: super::FormatRegistry

use std::marker::PhantomData;
use std::sync::Arc;

use elide_core::Result;
use elide_core::modality::Modality;

use super::Handler;
use super::document::{DocumentHandle, UntypedDocumentHandle};
use crate::content::ContentData;

/// Per-modality format loader.
///
/// A loader validates and parses raw content for modality `M`,
/// producing a handler that implements [`Handler<M>`]. Loaders are the
/// leaves the [`FormatRegistry`] composes: registering a format means
/// registering its loader.
///
/// # Implementing a third-party format
///
/// 1. Implement [`Handler<M>`] for the per-format handler type that owns
///    the parsed in-memory representation.
/// 2. Implement `Loader<M>` for a stateless type whose [`decode`]
///    validates raw [`ContentData`] and returns the handler.
/// 3. Build a [`Format`] with [`Format::new`], chain extensions /
///    content types as needed, and register it on a [`FormatRegistry`].
///
/// The registry erases `M` internally; third-party callers never touch
/// the object-safe surface.
///
/// [`Handler<M>`]: super::Handler
/// [`FormatRegistry`]: super::FormatRegistry
/// [`decode`]: Loader::decode
/// [`Format`]: super::Format
/// [`Format::new`]: super::Format::new
#[async_trait::async_trait]
pub trait Loader<M: Modality>: Send + Sync + 'static {
    /// The handler type this loader produces.
    type Handler: Handler<M>;

    /// Validate and parse the content, returning the loaded handler.
    ///
    /// # Errors
    ///
    /// Returns an error when the content is malformed for this format.
    async fn decode(&self, content: ContentData) -> Result<Self::Handler>;
}

/// Modality-erased loader the [`FormatRegistry`] holds behind `Arc`.
/// Adapts a per-modality [`Loader<M>`] into a uniform `decode` returning
/// an [`UntypedDocumentHandle`].
///
/// Crate-internal: every consumer goes through [`Format::decode`] or
/// [`FormatRegistry::decode`] instead.
///
/// [`FormatRegistry`]: super::FormatRegistry
/// [`Format::decode`]: super::Format::decode
/// [`FormatRegistry::decode`]: super::FormatRegistry::decode
#[async_trait::async_trait]
pub(crate) trait ErasedLoader: Send + Sync + 'static {
    async fn decode(&self, content: ContentData) -> Result<UntypedDocumentHandle>;
}

/// Erase a typed [`Loader<M>`] into an `Arc<dyn ErasedLoader>` the
/// registry can store. Called only by [`Format::new`].
///
/// [`Format::new`]: super::Format::new
pub(crate) fn erase<M, L>(loader: L) -> Arc<dyn ErasedLoader>
where
    M: Modality,
    L: Loader<M>,
{
    Arc::new(LoaderAdapter {
        loader,
        _phantom: PhantomData,
    })
}

/// Private wrapper holding a typed [`Loader<M>`] and implementing the
/// object-safe [`ErasedLoader`] surface. Constructed only via [`erase`].
struct LoaderAdapter<M: Modality, L: Loader<M>> {
    loader: L,
    _phantom: PhantomData<fn() -> M>,
}

#[async_trait::async_trait]
impl<M, L> ErasedLoader for LoaderAdapter<M, L>
where
    M: Modality,
    L: Loader<M>,
{
    async fn decode(&self, content: ContentData) -> Result<UntypedDocumentHandle> {
        let handler = self.loader.decode(content).await?;
        let format_id = Handler::format(&handler);
        let boxed: Box<dyn Handler<M>> = Box::new(handler);
        let handle = DocumentHandle::<M>::new(format_id, boxed);
        Ok(UntypedDocumentHandle::new(handle))
    }
}
