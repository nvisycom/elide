//! The [`DataReader`] trait — reading content at a location.

use std::future::Future;

use super::Modality;

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
/// Returns `None` when the location addresses nothing in this source
/// (out of range, a malformed location). The anonymizer treats that as
/// "skip this entity" rather than an error.
///
/// The read counterpart to [`DataWriter`](super::DataWriter), which
/// applies a [`Replacement`](Modality::Replacement) back at a location.
pub trait DataReader<M: Modality>: Send + Sync {
    /// The data at `location`, or `None` if it addresses nothing.
    fn read_at(&self, location: &M::Location) -> impl Future<Output = Option<M::Data>> + Send;
}
