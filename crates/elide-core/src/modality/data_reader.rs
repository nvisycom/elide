//! Reading traits: [`DataReader`] (random access at a location) and
//! [`StreamDataReader`] (sequential streaming + lift).

use super::{Chunk, Modality};
use crate::entity::Entity;
use crate::error::Result;

/// Reads the [`Data`] at a [`Location`] within some source.
///
/// Implemented by a modality's content holder (a text buffer, a decoded
/// image, a parsed document): the thing being redacted. The anonymizer
/// calls [`read_at`] once per entity to obtain just that entity's slice,
/// which it hands to the operator. This is what keeps operators pure and
/// modality-parametric: they never see the whole source, only the slice
/// the reader produces.
///
/// Returns `Ok(None)` when the location addresses nothing in this source
/// (out of range, a location that crosses a structural boundary); the
/// anonymizer treats that as "skip this entity". Returns `Err` when the
/// read itself fails: a malformed offset that lands mid-character, a
/// decode error. A codec-backed reader surfaces those loudly rather than
/// silently collapsing them to a miss.
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
    /// Data at `location`: `Ok(Some(data))` on a hit, `Ok(None)` when the
    /// location addresses nothing, `Err` when the read fails.
    async fn read_at(&self, location: &M::Location) -> Result<Option<M::Data>>;
}

/// Streams a source as [`Chunk`]s and lifts locations to source coordinates.
///
/// The sequential counterpart to [`DataReader`]: where `read_at` is
/// random access (give it a location, get that slice back), a
/// `StreamDataReader` walks the whole source front to back, yielding one
/// decoded [`Chunk`] at a time via [`read_next`]. The analyzer drives it
/// to feed recognizers chunk by chunk.
///
/// Recognizers see a chunk's decoded payload and emit entities whose
/// locations address *that chunk's* coordinate system. [`lift`] maps such
/// an entity back to a source-coordinate one, so everything downstream
/// (deduplication, anonymization, writing) speaks the source's own
/// coordinates. The default is the identity: a flat in-memory source
/// whose single chunk *is* the source needs no remapping. Sources whose
/// decoded chunks diverge from their bytes (escaped text, structured
/// documents) override it.
///
/// The source owns the streaming cursor: concurrent iteration of the
/// same source is not supported (only one `&mut self`).
///
/// [`Chunk`]: super::Chunk
/// [`read_next`]: StreamDataReader::read_next
/// [`lift`]: StreamDataReader::lift
#[async_trait::async_trait]
pub trait StreamDataReader<M: Modality>: Send {
    /// Advance the cursor and yield the next [`Chunk`], or `Ok(None)` at
    /// end-of-stream. Propagates the source's decode error.
    async fn read_next(&mut self) -> Result<Option<Chunk<M>>>;

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
