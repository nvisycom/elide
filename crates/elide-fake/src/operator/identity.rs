//! [`Identity`]: stable per-entity key used to seed the
//! generator RNG.
//!
//! Two-mention coreference (same real-world entity recognised
//! more than once in a document) must collapse to the same fake
//! value so a reader can follow it through the redacted text.
//! The RNG seed therefore keys on the entity's coreference id
//! when one is set, and on the entity's UUID bytes otherwise.

use std::hash::{Hash, Hasher};

use elide_core::entity::Entity;
use elide_core::modality::Modality;

/// Identity key used to seed the generator RNG. Prefers the
/// coreference id (shared across mentions of the same
/// real-world entity) and falls back to the entity's UUID bytes
/// when the recognizer did not resolve a cluster.
pub(super) enum Identity<'a> {
    /// Coreference cluster id — every mention of the same
    /// real-world entity shares this.
    Coref(&'a str),
    /// Per-entity UUID bytes — distinct per recognised mention
    /// when no coref is set.
    Uuid([u8; 16]),
}

impl<'a, M> From<&'a Entity<M>> for Identity<'a>
where
    M: Modality,
{
    fn from(entity: &'a Entity<M>) -> Self {
        match entity.coref.as_ref() {
            Some(coref) => Identity::Coref(coref.as_str()),
            None => Identity::Uuid(*entity.id.as_bytes()),
        }
    }
}

impl Hash for Identity<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Identity::Coref(s) => {
                0u8.hash(state);
                s.hash(state);
            }
            Identity::Uuid(bytes) => {
                1u8.hash(state);
                bytes.hash(state);
            }
        }
    }
}
