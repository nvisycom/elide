//! Decoding raw bytes into a typed handle, plus the erasure machinery
//! the registry stores.
//!
//! - [`Loader<M>`] — per-modality decoder a format implementation
//!   writes. Returns a concrete handler implementing
//!   [`Handler<M>`](super::Handler).
//! - [`DynHandler<M>`] — crate-private object-safe bridge over
//!   `Handler<M>` (boxes its RPITIT futures) so a
//!   [`DocumentHandle<M>`](super::document::DocumentHandle) can store
//!   `Box<dyn DynHandler<M>>`.
//! - [`ErasedLoader`] — modality-erased loader the
//!   [`CodecRegistry`](super::CodecRegistry) holds behind `Arc`.
//! - [`erase`] — bridge from a typed `Loader<M>` to
//!   `Arc<dyn ErasedLoader>`.

use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use veil_core::Error;
use veil_core::modality::{DataReader, DataWriter, Modality};
use veil_core::redaction::Redactions;

use super::capability::Chunk;
use super::Handler;
use crate::content::ContentData;
use super::document::{DocumentHandle, UntypedDocumentHandle};

/// A boxed, pinned future — the shape the object-safe bridges return so
/// `Handler`'s RPITIT futures can be stored behind a trait object.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Per-modality format loader.
///
/// A loader validates and parses raw content for modality `M`,
/// producing a handler that implements [`Handler<M>`](super::Handler).
/// Loaders are the leaves the [`CodecRegistry`](super::CodecRegistry)
/// composes — registering a format means registering its loader.
///
/// # Implementing a third-party format
///
/// 1. Implement [`Handler<M>`](super::Handler) for the per-format
///    handler type that owns the parsed in-memory representation.
/// 2. Implement `Loader<M>` for a stateless type whose
///    [`decode`](Loader::decode) validates raw [`ContentData`] and
///    returns the handler.
/// 3. Build a [`Format`](super::Format) with
///    [`Format::new`](super::Format::new), chain extensions / content
///    types as needed, and register it on a
///    [`CodecRegistry`](super::CodecRegistry).
///
/// The registry erases `M` internally; third-party callers never touch
/// the object-safe surface.
pub trait Loader<M: Modality>: Send + Sync + 'static {
    /// The handler type this loader produces.
    type Handler: Handler<M>;

    /// Validate and parse the content, returning the loaded handler.
    ///
    /// # Errors
    ///
    /// Returns an error when the content is malformed for this format.
    fn decode(
        &self,
        content: ContentData,
    ) -> impl Future<Output = Result<Self::Handler, Error>> + Send;
}

/// Object-safe bridge over [`Handler<M>`](super::Handler).
///
/// `Handler`'s async methods return `impl Future` (RPITIT), which is not
/// object-safe, so a [`DocumentHandle<M>`](super::document::DocumentHandle)
/// can't store `Box<dyn Handler<M>>`. This crate-private trait boxes the
/// futures; a blanket impl makes every `Handler` one automatically, so
/// the boxing is invisible at the public API.
pub(crate) trait DynHandler<M: Modality>: Send + Sync + 'static {
    fn encode(&self) -> Result<ContentData, Error>;

    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Chunk<M>>, Error>>;

    fn read_at<'a>(
        &'a self,
        location: &'a M::Location,
    ) -> BoxFuture<'a, Result<Option<M::Data>, Error>>;

    fn write_at(&mut self, redactions: Redactions<M>) -> BoxFuture<'_, Result<(), Error>>;

    fn lift_chunk(&self, chunk: &Chunk<M>, value_range: std::ops::Range<usize>)
    -> Option<M::Location>;
}

impl<M, H> DynHandler<M> for H
where
    M: Modality,
    H: Handler<M>,
{
    fn encode(&self) -> Result<ContentData, Error> {
        Handler::encode(self)
    }

    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Chunk<M>>, Error>> {
        Box::pin(Handler::next_chunk(self))
    }

    fn read_at<'a>(
        &'a self,
        location: &'a M::Location,
    ) -> BoxFuture<'a, Result<Option<M::Data>, Error>> {
        Box::pin(DataReader::read_at(self, location))
    }

    fn write_at(&mut self, redactions: Redactions<M>) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(DataWriter::write_at(self, redactions))
    }

    fn lift_chunk(
        &self,
        chunk: &Chunk<M>,
        value_range: std::ops::Range<usize>,
    ) -> Option<M::Location> {
        Handler::lift_chunk(self, chunk, value_range)
    }
}

/// Modality-erased loader the [`CodecRegistry`](super::CodecRegistry)
/// holds behind `Arc`. Adapts a per-modality [`Loader<M>`] into a
/// uniform `decode` returning an [`UntypedDocumentHandle`].
///
/// Crate-internal: every consumer goes through
/// [`Format::decode`](super::Format::decode) or
/// [`CodecRegistry::decode`](super::CodecRegistry::decode) instead.
pub(crate) trait ErasedLoader: Send + Sync + 'static {
    fn decode(&self, content: ContentData) -> BoxFuture<'_, Result<UntypedDocumentHandle, Error>>;
}

/// Erase a typed [`Loader<M>`] into an `Arc<dyn ErasedLoader>` the
/// registry can store. Called only by
/// [`Format::new`](super::Format::new).
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
    fn decode(&self, content: ContentData) -> BoxFuture<'_, Result<UntypedDocumentHandle, Error>> {
        Box::pin(async move {
            let handler = self.loader.decode(content).await?;
            let format_id = Handler::format(&handler);
            let boxed: Box<dyn DynHandler<M>> = Box::new(handler);
            let handle = DocumentHandle::<M>::new(format_id, boxed);
            Ok(UntypedDocumentHandle::new(handle))
        })
    }
}
