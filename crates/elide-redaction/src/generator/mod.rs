//! [`Generator`]: produce a synthetic replacement for an entity.

mod random;

use elide_core::entity::LabelRef;
use elide_core::modality::Modality;

pub use self::random::RandomToken;

/// Mints a synthetic replacement for an entity.
///
/// Owns *what* a synthetic value looks like, generic over the
/// [`Modality`] `M` so the same seam serves a fake string for text and,
/// later, a synthetic region for an image or a voice-converted clip for
/// audio — each producing that modality's [`Replacement`].
///
/// A generator need not be deterministic: a caller that needs the same
/// entity to render consistently across mentions layers a vault over the
/// generator, so per-cluster consistency comes from there, not from
/// `generate`. The `seed` is the cluster identity to derive from when
/// wanted (a coreference id, or the original value), and ignorable
/// otherwise.
///
/// [`Modality`]: elide_core::modality::Modality
/// [`Replacement`]: elide_core::modality::Modality::Replacement
pub trait Generator<M: Modality>: Send + Sync {
    /// Produce a synthetic replacement for an entity of `label`, with
    /// `seed` available as the cluster identity to derive from if wanted.
    fn generate(&self, label: &LabelRef, seed: &str) -> M::Replacement;
}
