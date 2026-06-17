//! The [`Deanonymizer`] trait — the optional reverse redaction direction.

use std::future::Future;

use crate::entity::Entity;
use crate::error::Error;
use crate::modality::Modality;
use crate::redaction::Anonymizer;

/// The reverse redaction direction: recovers the original data an
/// [`Anonymizer`] replaced.
///
/// A supertrait extension of [`Anonymizer`]: a `Deanonymizer` is always
/// also an `Anonymizer`, sharing the same [`id`](Anonymizer::id), and
/// only reversible operators (encrypt → decrypt) implement it. Like
/// `anonymize`, it is **pure** — it reads the entity and the
/// [`Replacement`](Modality::Replacement) and returns the recovered
/// [`Data`](Modality::Data), without mutating anything.
///
/// Recovery material may live inside the replacement itself
/// (self-contained, e.g. an AES-GCM ciphertext blob) or be looked up
/// out-of-band keyed by the entity. Returns `None` when this operator
/// cannot recover the original for the given replacement (e.g. the
/// replacement wasn't produced by it).
pub trait Deanonymizer<M: Modality>: Anonymizer<M> {
    /// Recover the original data for `entity` from its `replacement`,
    /// or `None` if it cannot be reversed.
    fn deanonymize(
        &self,
        entity: &Entity<M>,
        replacement: &M::Replacement,
    ) -> impl Future<Output = Result<Option<M::Data>, Error>> + Send;
}
