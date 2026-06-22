//! Decoding raw bytes into a typed handle, plus the erasure machinery
//! the registry stores.
//!
//! - [`Loader<M>`]: per-modality decoder a format implementation
//!   writes. Returns a concrete handler implementing [`Handler<M>`].
//! - [`DynHandler<M>`]: crate-private object-safe bridge over
//!   `Handler<M>` (boxes its RPITIT futures) so a [`DocumentHandle<M>`]
//!   can store `Box<dyn DynHandler<M>>`.
//! - [`ErasedLoader`]: modality-erased loader the [`FormatRegistry`]
//!   holds behind `Arc`.
//! - [`erase`]: bridge from a typed `Loader<M>` to
//!   `Arc<dyn ErasedLoader>`.
//!
//! [`Handler<M>`]: super::Handler
//! [`DocumentHandle<M>`]: super::document::DocumentHandle
//! [`FormatRegistry`]: super::FormatRegistry

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use elide_core::Result;
use elide_core::modality::{Chunk, DataReader, DataWriter, Modality};
use elide_core::redaction::Redactions;

use super::document::{DocumentHandle, UntypedDocumentHandle};
use super::{Container, Handler};
use crate::content::ContentData;

/// A boxed, pinned future: the shape the object-safe bridges return so
/// `Handler`'s RPITIT futures can be stored behind a trait object.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
pub trait Loader<M: Modality>: Send + Sync + 'static {
    /// The handler type this loader produces.
    type Handler: Handler<M>;

    /// Validate and parse the content, returning the loaded handler.
    ///
    /// # Errors
    ///
    /// Returns an error when the content is malformed for this format.
    fn decode(&self, content: ContentData) -> impl Future<Output = Result<Self::Handler>> + Send;
}

/// Object-safe bridge over [`Handler<M>`].
///
/// `Handler`'s async methods return `impl Future` (RPITIT), which is not
/// object-safe, so a [`DocumentHandle<M>`] can't store
/// `Box<dyn Handler<M>>`. This crate-private trait boxes the futures; a
/// blanket impl makes every `Handler` one automatically, so the boxing
/// is invisible at the public API.
///
/// [`Handler<M>`]: super::Handler
/// [`DocumentHandle<M>`]: super::document::DocumentHandle
pub(crate) trait DynHandler<M: Modality>: Send + Sync + 'static {
    fn encode(&self) -> Result<ContentData>;

    fn read_next(&mut self) -> BoxFuture<'_, Result<Option<Chunk<M>>>>;

    fn read_at<'a>(&'a self, location: &'a M::Location) -> BoxFuture<'a, Result<Option<M::Data>>>;

    fn write_at(&mut self, redactions: Redactions<M>) -> BoxFuture<'_, Result<()>>;

    fn lift(&self, chunk: &Chunk<M>, local: M::Location) -> Option<M::Location>;

    fn as_container_mut(&mut self) -> Option<&mut dyn Container>;
}

impl<M, H> DynHandler<M> for H
where
    M: Modality,
    H: Handler<M>,
{
    fn encode(&self) -> Result<ContentData> {
        Handler::encode(self)
    }

    fn read_next(&mut self) -> BoxFuture<'_, Result<Option<Chunk<M>>>> {
        Box::pin(Handler::read_next(self))
    }

    fn read_at<'a>(&'a self, location: &'a M::Location) -> BoxFuture<'a, Result<Option<M::Data>>> {
        Box::pin(DataReader::read_at(self, location))
    }

    fn write_at(&mut self, redactions: Redactions<M>) -> BoxFuture<'_, Result<()>> {
        Box::pin(DataWriter::write_at(self, redactions))
    }

    fn lift(&self, chunk: &Chunk<M>, local: M::Location) -> Option<M::Location> {
        Handler::lift(self, chunk, local)
    }

    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        Handler::as_container_mut(self)
    }
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
pub(crate) trait ErasedLoader: Send + Sync + 'static {
    fn decode(&self, content: ContentData) -> BoxFuture<'_, Result<UntypedDocumentHandle>>;
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

impl<M, L> ErasedLoader for LoaderAdapter<M, L>
where
    M: Modality,
    L: Loader<M>,
{
    fn decode(&self, content: ContentData) -> BoxFuture<'_, Result<UntypedDocumentHandle>> {
        Box::pin(async move {
            let handler = self.loader.decode(content).await?;
            let format_id = Handler::format(&handler);
            let boxed: Box<dyn DynHandler<M>> = Box::new(handler);
            let handle = DocumentHandle::<M>::new(format_id, boxed);
            Ok(UntypedDocumentHandle::new(handle))
        })
    }
}
