//! The [`DataReader`] trait — reading content at a location.

use std::future::Future;

use super::Modality;
use crate::error::Error;

/// Reads the [`Data`](Modality::Data) at a
/// [`Location`](Modality::Location) within some source.
///
/// Implemented by a modality's content holder (a text buffer, a decoded
/// image, a parsed document) — the thing being redacted. The anonymizer
/// calls [`read_at`](DataReader::read_at) once per entity to obtain just
/// that entity's slice, which it hands to the operator. This is what
/// keeps operators pure and modality-parametric: they never see the
/// whole source, only the slice the reader produces.
///
/// Returns `Ok(None)` when the location addresses nothing in this source
/// (out of range, a location that crosses a structural boundary); the
/// anonymizer treats that as "skip this entity". Returns `Err` when the
/// read itself fails — a malformed offset that lands mid-character, a
/// decode error. A codec-backed reader surfaces those loudly rather than
/// silently collapsing them to a miss.
///
/// The read counterpart to [`DataWriter`](super::DataWriter), which
/// applies a batch of replacements back into the source.
pub trait DataReader<M: Modality>: Send + Sync {
    /// The data at `location`: `Ok(Some(data))` on a hit, `Ok(None)` when
    /// the location addresses nothing, `Err` when the read fails.
    fn read_at(
        &self,
        location: &M::Location,
    ) -> impl Future<Output = Result<Option<M::Data>, Error>> + Send;
}
