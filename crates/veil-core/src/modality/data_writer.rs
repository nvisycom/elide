//! The [`DataWriter`] trait — applying a replacement at a location.

use std::future::Future;

use super::Modality;
use crate::error::Error;

/// Applies a [`Replacement`](Modality::Replacement) at a
/// [`Location`](Modality::Location) within some target.
///
/// The write counterpart to [`DataReader`](super::DataReader):
/// implemented by a modality's mutable content holder (a text buffer
/// being rewritten, an image being painted over), it takes the
/// instruction an operator produced and applies it back into the
/// document. This completes the redaction round-trip — read the value,
/// compute a replacement, write it — while keeping operators free of
/// format knowledge: the writer owns the *how* of applying each
/// modality's replacement.
///
/// Fails with an [`Error`] when the replacement cannot be applied (a
/// location out of range, an encoding failure). Applying many
/// replacements is the caller's loop; ordering (e.g. back-to-front for
/// text, to keep offsets valid) is the caller's concern.
pub trait DataWriter<M: Modality>: Send + Sync {
    /// Apply `replacement` at `location`.
    fn write_at(
        &mut self,
        location: &M::Location,
        replacement: &M::Replacement,
    ) -> impl Future<Output = Result<(), Error>> + Send;
}
