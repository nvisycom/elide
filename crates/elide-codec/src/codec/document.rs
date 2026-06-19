//! [`UntypedDocumentHandle`] (modality-erased handle the registry
//! returns) and [`DocumentHandle<M>`] (typed view used downstream of
//! decode).
//!
//! # Open two-tier handle shape
//!
//! The registry can't know up front which modality a decoded format
//! produces: that's a property of the format descriptor, resolved at
//! decode time. [`UntypedDocumentHandle`] is the registry-level return:
//! a [`FormatId`] plus the typed [`DocumentHandle<M>`] erased to
//! `Box<dyn Any>`. Unlike a closed per-modality enum, this supports any
//! [`Modality`], including custom ones a downstream crate defines, with
//! no central registry of kinds.
//!
//! Consumers commit to a modality via [`into`], which downcasts by
//! [`TypeId`] and yields the typed [`DocumentHandle<M>`], a wrapper that
//! owns the underlying handler and exposes the per-modality capability
//! surface.
//!
//! [`into`]: UntypedDocumentHandle::into
//! [`TypeId`]: std::any::TypeId

use std::any::{Any, type_name};
use std::fmt;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::text::Text;
use elide_core::modality::{Chunk, DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::redaction::Redactions;

use super::FormatId;
use super::loader::DynHandler;
use crate::content::ContentData;

/// Modality-erased handle the registry returns.
///
/// Carries a typed [`DocumentHandle<M>`] for some `M` plus the
/// [`FormatId`] of the producing loader.
///
/// Commit to a modality with [`into::<M>()`] (or peek with [`is`]) to
/// recover the typed [`DocumentHandle<M>`]. The downcast is by
/// [`TypeId`], so any registered modality works, built-in or custom.
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

/// Typed view of a single-modality handle.
///
/// Carries the [`FormatId`] alongside the handler so provenance can
/// always answer "what format is this?" without re-decoding.
///
/// Constructed by codec loaders, erased into an [`UntypedDocumentHandle`]
/// for registry return, then recovered with
/// [`UntypedDocumentHandle::into`]. Implements core's [`DataReader`] /
/// [`DataWriter`] for every modality by delegating to the underlying
/// handler, so any pipeline component can read from / write to a
/// codec-backed source through the same traits the toolkit bounds on.
///
/// [`DataReader`]: elide_core::modality::DataReader
/// [`DataWriter`]: elide_core::modality::DataWriter
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

    /// Serialize the current handler content back to [`ContentData`].
    ///
    /// # Errors
    ///
    /// Returns an error when the in-memory representation cannot be
    /// re-encoded.
    ///
    /// [`ContentData`]: crate::content::ContentData
    pub fn encode(&self) -> Result<ContentData> {
        self.handler.encode()
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
    async fn read_at(&self, location: &M::Location) -> Result<Option<M::Data>> {
        self.handler.read_at(location).await
    }
}

impl<M: Modality> DataWriter<M> for DocumentHandle<M> {
    async fn write_at(&mut self, redactions: Redactions<M>) -> Result<()> {
        self.handler.write_at(redactions).await
    }
}

impl StreamDataReader<Text> for DocumentHandle<Text> {
    async fn read_next(&mut self) -> Result<Option<Chunk<Text>>> {
        self.handler.read_next().await
    }

    /// Lift a text entity from chunk-local to source coordinates by
    /// mapping its `[start, end)` offset range through the handler's
    /// [`lift_chunk`]. Drops the entity when the range has no source
    /// pre-image.
    ///
    /// [`lift_chunk`]: crate::Handler::lift_chunk
    fn lift(&self, chunk: &Chunk<Text>, mut entity: Entity<Text>) -> Option<Entity<Text>> {
        let range = entity.location.start..entity.location.end;
        entity.location = self.handler.lift_chunk(chunk, range)?;
        Some(entity)
    }
}
