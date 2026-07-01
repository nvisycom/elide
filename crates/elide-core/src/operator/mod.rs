//! Operator contracts: how a detected entity is hidden.
//!
//! An operator reads an [`Entity`] and the [`Data`] under it and
//! *computes* a [`Replacement`] (mask, replace, hash, encrypt, blur, …)
//! without mutating anything; applying the replacement back into the
//! document is the codec's job. The forward direction is [`Operator`];
//! the optional reverse is [`ReversibleOperator`]. Every operator is an
//! [`Operator`]; only reversible ones (encrypt → decrypt) additionally
//! implement [`ReversibleOperator`].
//!
//! This module defines the operator contracts and the [`Redactions`]
//! batch they feed into. The redaction engine — the concrete operators,
//! the token vault they resolve through, and the label→operator registry
//! that selects them — lives in `elide-redaction`.
//!
//! [`Entity`]: crate::entity::Entity
//! [`Data`]: crate::modality::Modality::Data
//! [`Replacement`]: crate::modality::Modality::Replacement

mod leak_profile;
mod operator_id;
mod redactions;

pub use self::leak_profile::LeakProfile;
pub use self::operator_id::OperatorId;
pub use self::redactions::Redactions;
use crate::entity::Entity;
use crate::error::Result;
use crate::modality::Modality;

/// Redaction operator: computes what should replace an entity to remove
/// or obscure it.
///
/// Modelled on Presidio's anonymizer operators (replace, redact, mask,
/// hash, encrypt, keep, custom), generalised to be multimodal. Every
/// redaction operator implements this trait.
///
/// An operator is **pure**: it reads the entity and the [`Data`] under
/// it and returns a [`Replacement`] (the instruction for what to write)
/// *without* mutating anything. Applying the replacement back into the
/// document is the codec's job. This keeps operators free of format
/// knowledge (a mask works the same whether the text lives in a PDF or a
/// CSV), trivially testable, and cacheable. A reversible operator
/// (encrypt) additionally implements [`ReversibleOperator`]; an
/// irreversible one (mask, redact, hash) does not.
///
/// Generic over the [`Modality`] `M`: reads `M::Data`, returns
/// `M::Replacement`.
///
/// [`Data`]: Modality::Data
/// [`Replacement`]: Modality::Replacement
#[async_trait::async_trait]
pub trait Operator<M: Modality>: Send + Sync {
    /// This operator's identity (name + version).
    fn id(&self) -> OperatorId;

    /// How much this operator's output leaks about the original it replaces,
    /// for policy authoring and audit reporting.
    fn leak_profile(&self) -> LeakProfile;

    /// Compute the replacement for `entity`, reading its underlying `data`.
    /// Pure: produces the instruction, does not apply it.
    async fn anonymize(&self, entity: &Entity<M>, data: &M::Data) -> Result<M::Replacement>;
}

/// Reversible redaction operator: recovers the original data it replaced.
///
/// A supertrait extension of [`Operator`]: a `ReversibleOperator` is
/// always also an [`Operator`], sharing the same [`id`], and only
/// reversible operators (encrypt → decrypt) implement it. Like
/// `anonymize`, it is **pure**: it reads the entity and the
/// [`Replacement`] and returns the recovered [`Data`], without mutating
/// anything.
///
/// Recovery material may live inside the replacement itself
/// (self-contained, e.g. an AES-GCM ciphertext blob) or be looked up
/// out-of-band keyed by the entity. Returns `None` when this operator
/// cannot recover the original for the given replacement (e.g. the
/// replacement wasn't produced by it).
///
/// [`id`]: Operator::id
/// [`Replacement`]: Modality::Replacement
/// [`Data`]: Modality::Data
#[async_trait::async_trait]
pub trait ReversibleOperator<M: Modality>: Operator<M> {
    /// Recover the original data for `entity` from its `replacement`, or
    /// `None` if it cannot be reversed.
    async fn deanonymize(
        &self,
        entity: &Entity<M>,
        replacement: &M::Replacement,
    ) -> Result<Option<M::Data>>;
}
