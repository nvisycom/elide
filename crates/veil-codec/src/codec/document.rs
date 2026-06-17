//! [`UntypedDocumentHandle`] (modality-erased handle the registry
//! returns) and [`DocumentHandle<M>`] (typed view used downstream of
//! decode).
//!
//! # Open two-tier handle shape
//!
//! The registry can't know up front which modality a decoded format
//! produces — that's a property of the format descriptor, resolved at
//! decode time. [`UntypedDocumentHandle`] is the registry-level return:
//! a [`FormatId`] plus the typed [`DocumentHandle<M>`] erased to
//! `Box<dyn Any>`. Unlike a closed per-modality enum, this supports any
//! [`Modality`] — including custom ones a downstream crate defines —
//! with no central registry of kinds.
//!
//! Consumers commit to a modality via [`into`], which downcasts by
//! [`TypeId`] and yields the typed [`DocumentHandle<M>`] — a wrapper that
//! owns the underlying handler and exposes the per-modality capability
//! surface.
//!
//! [`into`]: UntypedDocumentHandle::into
//! [`TypeId`]: std::any::TypeId

use std::any::{Any, type_name};
use std::fmt;
use std::ops::Range;

use veil_core::modality::{DataReader, DataWriter, Modality};
use veil_core::redaction::Redactions;

use super::FormatId;
use super::loader::DynHandler;

/// Modality-erased handle the registry returns, carrying a typed
/// [`DocumentHandle<M>`] for some `M` plus the [`FormatId`] of the
/// producing loader.
///
/// Commit to a modality with [`into::<M>()`] (or peek with [`is`]) to
/// recover the typed [`DocumentHandle<M>`]. The downcast is by
/// [`TypeId`], so any registered modality works — built-in or custom.
///
/// [`into::<M>()`]: Self::into
/// [`is`]: Self::is
/// [`TypeId`]: std::any::TypeId
pub struct UntypedDocumentHandle {
    format_id: FormatId,
    handle: Box<dyn Any + Send + Sync>,
}

impl UntypedDocumentHandle {
    /// Erase a typed [`DocumentHandle<M>`] into the untyped form.
    pub fn new<M: Modality>(handle: DocumentHandle<M>) -> Self {
        Self {
            format_id: handle.format_id.clone(),
            handle: Box::new(handle),
        }
    }

    /// The [`FormatId`] of the loader that produced this handle.
    pub fn format_id(&self) -> &FormatId {
        &self.format_id
    }

    /// Whether this handle carries modality `M`.
    pub fn is<M: Modality>(&self) -> bool {
        self.handle.is::<DocumentHandle<M>>()
    }

    /// Consume self, returning the typed [`DocumentHandle<M>`] if this
    /// handle carries modality `M`, or the untyped handle back on a
    /// mismatch so the caller can try another modality.
    ///
    /// # Errors
    ///
    /// Returns `Err(self)` when the carried modality is not `M`.
    pub fn into<M: Modality>(self) -> Result<DocumentHandle<M>, Self> {
        match self.handle.downcast::<DocumentHandle<M>>() {
            Ok(handle) => Ok(*handle),
            Err(handle) => Err(Self {
                format_id: self.format_id,
                handle,
            }),
        }
    }
}

impl fmt::Debug for UntypedDocumentHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UntypedDocumentHandle")
            .field("format_id", &self.format_id)
            .finish_non_exhaustive()
    }
}

/// Typed view of a single-modality handle. Carries the [`FormatId`]
/// alongside the handler so provenance can always answer "what format is
/// this?" without re-decoding.
///
/// Constructed by codec loaders, erased into an [`UntypedDocumentHandle`]
/// for registry return, then recovered with
/// [`UntypedDocumentHandle::into`]. Implements core's [`DataReader`] /
/// [`DataWriter`] for every modality by delegating to the underlying
/// handler, so any pipeline component can read from / write to a
/// codec-backed source through the same traits the toolkit bounds on.
///
/// [`DataReader`]: veil_core::modality::DataReader
/// [`DataWriter`]: veil_core::modality::DataWriter
pub struct DocumentHandle<M: Modality> {
    format_id: FormatId,
    handler: Box<dyn DynHandler<M>>,
}

impl<M: Modality> DocumentHandle<M> {
    /// Wrap a handler and a format id into a typed handle. Used by codec
    /// loaders; the handler is boxed through the crate-private
    /// object-safe bridge.
    pub(crate) fn new(format_id: FormatId, handler: Box<dyn DynHandler<M>>) -> Self {
        Self { format_id, handler }
    }

    /// The [`FormatId`] of the producing loader.
    pub fn format_id(&self) -> &FormatId {
        &self.format_id
    }

    /// Advance the streaming cursor and yield the next chunk, or `None`
    /// at end-of-stream.
    ///
    /// # Errors
    ///
    /// Propagates the handler's decode error.
    pub async fn next_chunk(&mut self) -> Result<Option<super::Chunk<M>>, veil_core::Error> {
        self.handler.next_chunk().await
    }

    /// Serialize the current handler content back to [`ContentData`].
    ///
    /// # Errors
    ///
    /// Returns an error when the in-memory representation cannot be
    /// re-encoded.
    ///
    /// [`ContentData`]: crate::content::ContentData
    pub fn encode(&self) -> Result<crate::content::ContentData, veil_core::Error> {
        self.handler.encode()
    }

    /// Translate a value-range inside a chunk's decoded payload back to a
    /// source-coordinate `M::Location`, or `None` when it has no source
    /// pre-image. See [`Handler::lift_chunk`].
    ///
    /// [`Handler::lift_chunk`]: crate::Handler::lift_chunk
    pub fn lift_chunk(
        &self,
        chunk: &super::Chunk<M>,
        value_range: Range<usize>,
    ) -> Option<M::Location> {
        self.handler.lift_chunk(chunk, value_range)
    }
}

impl<M: Modality> fmt::Debug for DocumentHandle<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentHandle")
            .field("format_id", &self.format_id)
            .field("modality", &type_name::<M>())
            .finish()
    }
}

impl<M: Modality> DataReader<M> for DocumentHandle<M> {
    async fn read_at(&self, location: &M::Location) -> Result<Option<M::Data>, veil_core::Error> {
        self.handler.read_at(location).await
    }
}

impl<M: Modality> DataWriter<M> for DocumentHandle<M> {
    async fn write_at(&mut self, redactions: Redactions<M>) -> Result<(), veil_core::Error> {
        self.handler.write_at(redactions).await
    }
}
