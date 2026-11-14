//! Reading traits: [`DataReader`] (random access at a location) and
//! [`ChunkReader`] (every decoded chunk, and the lift back to source).

use super::{Chunk, Modality};
use crate::entity::Entity;
use crate::error::Result;

/// Reads the [`Data`] at a [`Location`] within the content it decodes.
///
/// Implemented by a modality's content holder (a text buffer, a decoded
/// image, a parsed document): the thing being redacted. The anonymizer
/// calls [`read_at`] once per entity to obtain just that entity's slice,
/// which it hands to the operator. This is what keeps operators pure and
/// modality-parametric: they never see the whole source, only the slice
/// the reader produces.
///
/// A reader accepts every location its [`DataWriter`] does, so whatever can
/// be redacted can be read first. A location that addresses nothing in this
/// source (out of range, splitting a character, a reviewer's mark the source
/// cannot resolve) is an error, never a miss: the anonymizer would otherwise
/// skip the entity and leave it unredacted.
///
/// The read counterpart to [`DataWriter`], which applies a batch of
/// replacements back into the source.
///
/// [`Data`]: Modality::Data
/// [`Location`]: Modality::Location
/// [`read_at`]: DataReader::read_at
/// [`DataWriter`]: super::DataWriter
#[async_trait::async_trait]
pub trait DataReader<M: Modality>: Send + Sync {
    /// The data at `location`.
    ///
    /// # Errors
    ///
    /// [`MalformedInput`] if `location` addresses nothing in this source;
    /// otherwise the source's own read error.
    ///
    /// [`MalformedInput`]: crate::ErrorKind::MalformedInput
    async fn read_at(&self, location: &M::Location) -> Result<M::Data>;
}

/// Decodes a source into [`Chunk`]s and lifts locations to source coordinates.
///
/// The whole-source counterpart to [`DataReader`]: where `read_at` is random
/// access (give it a location, get that slice back), [`chunks`] hands back every
/// decoded [`Chunk`] the source holds, front to back, in one call. The analyzer
/// feeds recognizers chunk by chunk from that.
///
/// A source's chunks are already materialized when it is decoded, so [`chunks`]
/// is a plain `&self` read of an in-memory collection, not a lazy cursor:
/// borrowing it does not lock the source, so many sources' chunks can be gathered
/// together (the batched analysis path relies on this).
///
/// Recognizers see a chunk's decoded payload and emit entities whose locations
/// address *that chunk's* coordinate system. [`lift`] maps such an entity back to
/// a source-coordinate one, so everything downstream (the detection layers,
/// anonymization, writing) speaks the source's own coordinates. The default is
/// the identity: a flat in-memory source whose single chunk *is* the source needs
/// no remapping. Sources whose decoded chunks diverge from their bytes (escaped
/// text, structured documents) override it.
///
/// [`Chunk`]: super::Chunk
/// [`chunks`]: ChunkReader::chunks
/// [`lift`]: ChunkReader::lift
pub trait ChunkReader<M: Modality>: Send {
    /// Every decoded [`Chunk`] the source holds, front to back.
    ///
    /// # Errors
    ///
    /// Propagates the source's decode error.
    fn chunks(&self) -> Result<Vec<Chunk<M>>>;

    /// Map `entity`, whose location addresses `chunk`'s decoded payload,
    /// to a source-coordinate entity.
    ///
    /// Returns `None` when the entity's location has no source pre-image
    /// (out of bounds, inside an escape pair, or a modality with no
    /// meaningful mapping); the caller drops it. The default is the
    /// identity: the entity passes through unchanged.
    fn lift(&self, _chunk: &Chunk<M>, entity: Entity<M>) -> Option<Entity<M>> {
        Some(entity)
    }
}
