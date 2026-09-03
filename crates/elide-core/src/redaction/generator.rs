//! [`Generator`]: mint a synthetic replacement for an entity.

use crate::entity::Entity;
use crate::modality::Modality;

/// Mints a synthetic replacement for an entity.
///
/// Owns *what* a synthetic value looks like, the seam a pseudonymizing
/// operator delegates to. Generic over the [`Modality`] `M` so the same seam
/// serves a fake string for text and, later, a synthetic region for an image or
/// a voice-converted clip for audio, each producing that modality's
/// [`Replacement`].
///
/// Mirrors [`Operator::anonymize`](super::Operator::anonymize): it reads the
/// whole `entity` and the `data` under it, so a generator can dispatch on the
/// entity's [`label`](Entity::label), derive per-cluster consistency from its
/// [`coref`](Entity::coref), and pattern-preserve the original value it reads
/// from `data`. Unlike an operator it is **pure and infallible**: no `Result`,
/// no I/O, just a value.
///
/// A generator need not be deterministic on its own: a caller that needs the
/// same real-world entity to render consistently across mentions layers a vault
/// over the generator, so per-cluster consistency comes from there. When the
/// generator *is* deterministic, deriving from the entity's coreference makes
/// coreferent mentions collapse to one value before the vault even sees them.
///
/// [`Modality`]: crate::modality::Modality
/// [`Replacement`]: crate::modality::Modality::Replacement
pub trait Generator<M: Modality>: Send + Sync {
    /// Mint a synthetic replacement for `entity`, reading its underlying `data`
    /// (available to pattern-preserve the original, when the generator wants to).
    fn generate(&self, entity: &Entity<M>, data: &M::Data) -> M::Replacement;
}
