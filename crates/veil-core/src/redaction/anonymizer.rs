//! The [`Anonymizer`] trait — the forward redaction direction — and the
//! [`LeakProfile`] describing how much its output leaks.

use std::future::Future;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::entity::Entity;
use crate::error::Error;
use crate::modality::Modality;
use crate::redaction::OperatorId;

/// What a redacted output leaks about the original it replaced.
///
/// Variants are ordered from most-leaky to least-leaky, so
/// `Recoverable < Partial < Irrecoverable`. Surfaced through
/// [`Anonymizer::leak_profile`] for policy authoring and audit
/// reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum LeakProfile {
    /// The original value is recoverable from the output given the
    /// right metadata (encryption key, token vault, pseudonym map, or
    /// the candidate entity list against a hash).
    Recoverable,
    /// The original value is gone, but observable shape leaks:
    /// position, length, bounding box, cell coordinates, or a known
    /// silence on the timeline.
    Partial,
    /// No trace of the original value or its shape remains in the
    /// output.
    Irrecoverable,
}

/// The forward redaction direction: computes what should replace an
/// entity to remove or obscure it.
///
/// Modelled on Presidio's anonymizer operators (replace, redact, mask,
/// hash, encrypt, keep, custom), generalised to be multimodal. Every
/// redaction operator implements this trait.
///
/// An anonymizer is **pure**: it reads the entity and the
/// [`Data`](Modality::Data) under it and returns a
/// [`Replacement`](Modality::Replacement) — the instruction for what to
/// write — *without* mutating anything. Applying the replacement back
/// into the document is the codec's job. This keeps operators free of
/// format knowledge (a `Mask` works the same whether the text lives in a
/// PDF or a CSV), trivially testable, and cacheable. A reversible
/// operator (encrypt) additionally implements
/// [`Deanonymizer`](crate::redaction::Deanonymizer); an irreversible one
/// (mask, redact, hash) does not.
///
/// Generic over the [`Modality`] `M`: reads `M::Data`, returns
/// `M::Replacement`.
pub trait Anonymizer<M: Modality>: Send + Sync {
    /// This operator's identity (name + version).
    fn id(&self) -> OperatorId;

    /// How much this operator's output leaks about the original it
    /// replaces, for policy authoring and audit reporting.
    fn leak_profile(&self) -> LeakProfile;

    /// Compute the replacement for `entity`, reading its underlying
    /// `data`. Pure: produces the instruction, does not apply it.
    fn anonymize(
        &self,
        entity: &Entity<M>,
        data: &M::Data,
    ) -> impl Future<Output = Result<M::Replacement, Error>> + Send;
}
