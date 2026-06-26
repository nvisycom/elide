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
use elide_core::modality::{Chunk, DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::operator::Redactions;

use super::FormatId;
use super::loader::DynHandler;
use crate::codec::Container;
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
    handle: Box<dyn ErasedHandle>,
}

/// The modality-independent surface of a [`DocumentHandle<M>`], so an
/// [`UntypedDocumentHandle`] can re-encode or reach a document's container
/// parts without first committing to a modality. `Any` keeps the typed
/// downcast (`into`/`is`/`take`/`downcast_mut`) available.
trait ErasedHandle: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync>;
    fn encode(&self) -> Result<ContentData>;
    fn as_container_mut(&mut self) -> Option<&mut dyn Container>;
}

impl<M: Modality> ErasedHandle for DocumentHandle<M> {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
        self
    }
    fn encode(&self) -> Result<ContentData> {
        DocumentHandle::encode(self)
    }
    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        DocumentHandle::as_container_mut(self)
    }
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
        self.handle.as_any().is::<DocumentHandle<M>>()
    }

    /// Consume self, returning the typed [`DocumentHandle<M>`] if this
    /// handle carries modality `M`, or the untyped handle back on a
    /// mismatch so the caller can try another modality.
    ///
    /// # Errors
    ///
    /// Returns `Err(self)` when the carried modality is not `M`.
    pub fn into<M: Modality>(self) -> Result<DocumentHandle<M>, Self> {
        if !self.is::<M>() {
            return Err(self);
        }
        // The `is::<M>` check just passed, so the downcast holds.
        Ok(*self
            .handle
            .into_any()
            .downcast::<DocumentHandle<M>>()
            .unwrap_or_else(|_| unreachable!("is::<M> guaranteed the modality")))
    }

    /// Borrow the typed [`DocumentHandle<M>`] mutably if this handle carries
    /// modality `M`, else `None`. For reading from / detecting over a handle
    /// without consuming it.
    pub fn downcast_mut<M: Modality>(&mut self) -> Option<&mut DocumentHandle<M>> {
        self.handle.as_any_mut().downcast_mut::<DocumentHandle<M>>()
    }

    /// Re-encode the carried handle back to [`ContentData`], without
    /// committing to a modality.
    ///
    /// # Errors
    ///
    /// Returns an error when the in-memory representation cannot be
    /// re-encoded.
    ///
    /// [`ContentData`]: crate::content::ContentData
    pub fn encode(&self) -> Result<ContentData> {
        self.handle.encode()
    }

    /// This document as a [`Container`] of cross-modality sub-parts, if it
    /// is one. `None` for a plain single-modality format. Reaches the parts
    /// without committing to a modality.
    ///
    /// [`Container`]: crate::Container
    pub fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        self.handle.as_container_mut()
    }

    /// Move the typed [`DocumentHandle<M>`] out from behind a `&mut`,
    /// leaving the untyped handle empty.
    ///
    /// For a caller that holds an `&mut UntypedDocumentHandle`, must run a
    /// consuming operation on the typed handle (which takes the handle by
    /// value), and then writes the result back with [`new`]. `None` on a
    /// modality mismatch, leaving the handle untouched.
    ///
    /// [`new`]: Self::new
    pub fn take<M: Modality>(&mut self) -> Option<DocumentHandle<M>> {
        if !self.is::<M>() {
            return None;
        }
        // The `is::<M>` check just passed, so the downcast holds.
        let handle = std::mem::replace(&mut self.handle, Box::new(EmptyHandle));
        Some(
            *handle
                .into_any()
                .downcast::<DocumentHandle<M>>()
                .unwrap_or_else(|_| unreachable!("is::<M> guaranteed the modality")),
        )
    }
}

/// Placeholder content left behind by [`UntypedDocumentHandle::take`]; never
/// observed, since `take` immediately overwrites the slot or the caller
/// writes a fresh handle back.
struct EmptyHandle;

impl ErasedHandle for EmptyHandle {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn into_any(self: Box<Self>) -> Box<dyn Any + Send + Sync> {
        self
    }
    fn encode(&self) -> Result<ContentData> {
        unreachable!("encode on an emptied UntypedDocumentHandle")
    }
    fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        None
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

    /// This document as a [`Container`] of cross-modality sub-parts, if it
    /// is one. `None` for a plain single-modality format.
    ///
    /// [`Container`]: crate::Container
    pub fn as_container_mut(&mut self) -> Option<&mut dyn Container> {
        self.handler.as_container_mut()
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

impl<M: Modality> StreamDataReader<M> for DocumentHandle<M> {
    async fn read_next(&mut self) -> Result<Option<Chunk<M>>> {
        self.handler.read_next().await
    }

    /// Lift an entity from chunk-local to source coordinates by promoting
    /// its location through the handler's [`lift`]. The entity's location
    /// *is* the recognizer's chunk-local finding; `lift` rebases it onto
    /// the chunk's origin (a text offset add, a JSON escape walk, a cell's
    /// row/column). Drops the entity when the location has no source
    /// pre-image.
    ///
    /// [`lift`]: crate::Handler::lift
    fn lift(&self, chunk: &Chunk<M>, mut entity: Entity<M>) -> Option<Entity<M>> {
        entity.location = self.handler.lift(chunk, entity.location)?;
        Some(entity)
    }
}
