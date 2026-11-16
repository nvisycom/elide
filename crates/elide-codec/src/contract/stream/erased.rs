//! Modality erasure for a document's streams: [`StreamMut`] (a borrowed stream
//! of any modality, what a document lends the engine) and [`TypedStream`] (the
//! concrete, `Sized` handle a pipeline drives once it knows the modality).
//!
//! A format's document owns its streams as typed fields and lends each one as a
//! [`StreamMut`]. The engine recovers a stream only by modality
//! ([`StreamMut::downcast_mut`]); the document itself never looks a stream up,
//! it reads its own fields.

use std::any::Any;
use std::fmt;

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::{Chunk, ChunkReader, DataReader, DataWriter, Modality};
use elide_core::redaction::Redactions;

use super::{FlaggedItem, Stream};
use crate::contract::FormatId;

/// The function that recovers a [`StreamMut`]'s stream as a `dyn Stream<M>`.
type Cast<M> = fn(&mut (dyn Any + Send + Sync)) -> &mut dyn Stream<M>;

/// A borrowed stream of any modality, as a document lends it to the engine.
///
/// It holds the stream type-erased together with the function that recovers it
/// as a [`Stream<M>`] of its modality. A function pointer outlives any borrow,
/// so it erases into `Any` while the stream itself stays borrowed; the engine's
/// [`downcast_mut`](Self::downcast_mut) by modality is the only downcast.
pub struct StreamMut<'a> {
    stream: &'a mut (dyn Any + Send + Sync),
    /// A [`Cast<M>`] for the stream's modality `M`.
    cast: Box<dyn Any + Send + Sync>,
}

impl<'a> StreamMut<'a> {
    /// Lend `stream`, a [`Stream`] of modality `M`.
    pub fn new<M: Modality, S: Stream<M>>(stream: &'a mut S) -> Self {
        Self {
            stream,
            cast: Box::new(cast::<M, S> as Cast<M>),
        }
    }

    /// Whether this stream carries modality `M`.
    pub fn is<M: Modality>(&self) -> bool {
        self.cast.is::<Cast<M>>()
    }

    /// The stream as a [`TypedStream<M>`] for the rest of its borrow, if it
    /// carries modality `M`, else `None`.
    pub fn downcast<M: Modality>(self) -> Option<TypedStream<'a, M>> {
        let cast = *self.cast.downcast_ref::<Cast<M>>()?;
        Some(TypedStream {
            stream: cast(self.stream),
        })
    }

    /// The stream as a [`TypedStream<M>`] if it carries modality `M`, else
    /// `None`.
    pub fn downcast_mut<M: Modality>(&mut self) -> Option<TypedStream<'_, M>> {
        let cast = *self.cast.downcast_ref::<Cast<M>>()?;
        Some(TypedStream {
            stream: cast(&mut *self.stream),
        })
    }
}

impl fmt::Debug for StreamMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamMut").finish_non_exhaustive()
    }
}

/// Recover a stream [`StreamMut::new`] erased from an `S`.
fn cast<M: Modality, S: Stream<M>>(stream: &mut (dyn Any + Send + Sync)) -> &mut dyn Stream<M> {
    stream
        .downcast_mut::<S>()
        .expect("a StreamMut casts with the type it was built from")
}

/// A borrowed stream of modality `M`: the concrete, `Sized` handle a pipeline
/// drives.
///
/// It forwards [`ChunkReader`] / [`DataReader`] / [`DataWriter`] to the stream,
/// so a caller never has to name a trait object (which would trip the
/// compiler's `Send` inference through an `async` boundary).
pub struct TypedStream<'a, M: Modality> {
    stream: &'a mut dyn Stream<M>,
}

impl<M: Modality> TypedStream<'_, M> {
    /// The [`FormatId`] of the format this stream represents.
    pub fn format(&self) -> FormatId {
        self.stream.format()
    }

    /// The items of hidden content the stream's format labels; see
    /// [`Stream::flagged`].
    ///
    /// # Errors
    ///
    /// Returns the source's decode error.
    pub fn flagged(&self) -> Result<Vec<FlaggedItem<M>>> {
        self.stream.flagged()
    }

    /// `location` in the form the stream reads and writes; see
    /// [`Stream::resolve`].
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if `location` is a mark the stream cannot resolve.
    ///
    /// [`MalformedInput`]: elide_core::ErrorKind::MalformedInput
    pub fn resolve(&self, location: M::Location) -> Result<M::Location> {
        self.stream.resolve(location)
    }
}

#[async_trait::async_trait]
impl<M: Modality> DataReader<M> for TypedStream<'_, M> {
    async fn read_at(&self, location: &M::Location) -> Result<M::Data> {
        self.stream.read_at(location).await
    }
}

#[async_trait::async_trait]
impl<M: Modality> DataWriter<M> for TypedStream<'_, M> {
    async fn write_at(&mut self, redactions: Redactions<M>) -> Result<()> {
        self.stream.write_at(redactions).await
    }
}

impl<M: Modality> ChunkReader<M> for TypedStream<'_, M> {
    fn chunks(&self) -> Result<Vec<Chunk<M>>> {
        self.stream.chunks()
    }

    fn lift(&self, chunk: &Chunk<M>, mut entity: Entity<M>) -> Option<Entity<M>> {
        entity.location = self.stream.lift(chunk, entity.location)?;
        Some(entity)
    }
}
