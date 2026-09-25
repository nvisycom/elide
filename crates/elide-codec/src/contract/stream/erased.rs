//! Modality erasure for stream parts: [`ErasedStream`] (what a
//! [`DocumentPart::Stream`](super::super::DocumentPart::Stream) holds) and
//! [`TypedStream`] (the concrete, `Sized` handle a redaction pipeline drives).

use std::any::Any;
use std::fmt;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{Chunk, DataReader, DataWriter, Modality, StreamDataReader};
use elide_core::redaction::Redactions;

use super::Stream;
use crate::content::ContentData;
use crate::contract::FormatId;

/// A modality-erased [`Stream<M>`], so a [`Document`](super::super::Document)'s
/// stream parts of different modalities live in one `Vec`. Recovered by
/// [`TypeId`] — the `Any` supertrait allows the downcast.
///
/// [`TypeId`]: std::any::TypeId
pub struct ErasedStream {
    format_id: FormatId,
    stream: Box<dyn ErasedStreamObj>,
}

/// The modality-independent surface an [`ErasedStream`] exposes: re-encode this
/// stream's own bytes, without committing to a modality. `Any` keeps the typed
/// downcast available.
trait ErasedStreamObj: Any + Send + Sync {
    fn encode(&self) -> Result<ContentData>;
}

/// A typed stream wrapper (the erasure target), holding one modality's
/// [`Stream<M>`]. It is the concrete, `Sized` handle a redaction pipeline drives:
/// it forwards [`StreamDataReader`] / [`DataReader`] / [`DataWriter`] to the boxed
/// stream, so a caller never has to name a trait object (which would trip the
/// compiler's `Send` inference through an `async` boundary).
pub struct TypedStream<M: Modality> {
    stream: Box<dyn Stream<M>>,
}

impl<M: Modality> TypedStream<M> {
    /// The [`FormatId`] of the format this stream represents.
    pub fn format(&self) -> FormatId {
        self.stream.format()
    }
}

impl<M: Modality> ErasedStreamObj for TypedStream<M> {
    fn encode(&self) -> Result<ContentData> {
        self.stream.encode()
    }
}

#[async_trait::async_trait]
impl<M: Modality> DataReader<M> for TypedStream<M> {
    async fn read_at(&self, location: &M::Location) -> Result<Option<M::Data>> {
        self.stream.read_at(location).await
    }
}

#[async_trait::async_trait]
impl<M: Modality> DataWriter<M> for TypedStream<M> {
    async fn write_at(&mut self, redactions: Redactions<M>) -> Result<()> {
        self.stream.write_at(redactions).await
    }
}

#[async_trait::async_trait]
impl<M: Modality> StreamDataReader<M> for TypedStream<M> {
    async fn read_next(&mut self) -> Result<Option<Chunk<M>>> {
        Stream::read_next(&mut *self.stream).await
    }

    fn lift(&self, chunk: &Chunk<M>, mut entity: Entity<M>) -> Option<Entity<M>> {
        entity.location = Stream::lift(&*self.stream, chunk, entity.location)?;
        Some(entity)
    }
}

impl ErasedStream {
    /// Erase a typed [`Stream<M>`] into the modality-independent form.
    pub fn new<M: Modality>(format_id: FormatId, stream: Box<dyn Stream<M>>) -> Self {
        Self {
            format_id,
            stream: Box::new(TypedStream { stream }),
        }
    }

    /// The [`FormatId`] of the loader that produced this stream.
    pub fn format_id(&self) -> &FormatId {
        &self.format_id
    }

    /// Whether this stream carries modality `M`.
    pub fn is<M: Modality>(&self) -> bool {
        (&*self.stream as &dyn Any).is::<TypedStream<M>>()
    }

    /// Borrow the typed stream mutably if this stream carries modality `M`, else
    /// `None`. The returned [`TypedStream<M>`] is the concrete, `Sized` handle a
    /// pipeline drives (it forwards the read/redact stream traits), so no caller
    /// names a `dyn Stream<M>` trait object across an `async` boundary.
    pub fn downcast_mut<M: Modality>(&mut self) -> Option<&mut TypedStream<M>> {
        (&mut *self.stream as &mut dyn Any).downcast_mut::<TypedStream<M>>()
    }

    /// Re-encode this stream's own bytes, without committing to a modality.
    ///
    /// # Errors
    ///
    /// Returns an error when the in-memory representation cannot be re-encoded.
    pub fn encode(&self) -> Result<ContentData> {
        self.stream.encode()
    }
}

impl fmt::Debug for ErasedStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErasedStream")
            .field("format_id", &self.format_id)
            .finish_non_exhaustive()
    }
}
