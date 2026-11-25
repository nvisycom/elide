//! [`Seal`], [`SealKey`], and [`Sealable`]: what a report, a review, or an
//! artifact set must match to be read back.

use std::fmt;

use elide_core::entity::audit::AuditHash;
use elide_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::PartId;

/// A secret key for keyed seals.
///
/// Set on a [`Restorer`](crate::Restorer) with
/// [`with_seal_key`](crate::Restorer::with_seal_key), or on an orchestrator
/// with [`with_seal_key`](crate::Orchestrator::with_seal_key), it makes every
/// seal the restorer makes a message authentication code: a seal only a holder
/// of the key can produce, so it can be stored beside what it covers. Keep it
/// where the carrier cannot read it. Its `Debug` output does not show it.
#[derive(Clone)]
pub struct SealKey([u8; 32]);

impl SealKey {
    /// A key of the 32 bytes `bytes`, which should be uniformly random.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for SealKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SealKey(..)")
    }
}

/// What a serialized [`Report`], [`Review`], or [`ArtifactSet`] must match to
/// be read back.
///
/// Each entity's audit trail verifies on its own, but whoever carries a report
/// can still rewrite an entity whole, cut a trail back to an earlier event,
/// change a label or location beside its trail, or drop an entity, and every
/// remaining trail stays intact; a review can lose an edit on the way, and an
/// artifact set can change what re-analysis reads. A seal catches each of
/// these: it is the digest of the whole value when it was written.
///
/// A seal is plain data, and serializes as `{ "digest": hex }` or
/// `{ "mac": hex }`. Make one with [`Restorer::seal`] and read the value back
/// against it with [`Restorer::restore`]; the restorer's configuration picks
/// the scheme for both, so the two always agree:
///
/// - Without a [`SealKey`], a [`Digest`](Self::Digest): unkeyed, so anyone can
///   compute one, and it protects only if it is stored where the carrier cannot
///   write: a signed log, a write-once bucket, a timestamping service.
/// - With one, a [`Mac`](Self::Mac): only a holder of the key can compute it,
///   so it can be stored beside what it covers, as long as the carrier does
///   not hold the key.
///
/// The seal never chooses its own scheme: a restorer with a key accepts only a
/// `Mac`, so a carrier cannot swap one for a `Digest` it computed itself.
///
/// [`Restorer::seal`]: crate::Restorer::seal
/// [`Restorer::restore`]: crate::Restorer::restore
///
/// [`Report`]: crate::Report
/// [`Review`]: crate::Review
/// [`ArtifactSet`]: crate::ArtifactSet
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum Seal {
    /// The unkeyed digest of the value.
    Digest(AuditHash),
    /// The digest of the value keyed with a [`SealKey`].
    Mac(AuditHash),
}

impl Seal {
    /// The seal of `value`: keyed with `key` when there is one, unkeyed
    /// otherwise.
    pub(crate) fn new<T: Sealable>(value: &T, key: Option<&SealKey>) -> Self {
        match key {
            Some(key) => Self::Mac(AuditHash::of_fields_keyed(&key.0, fields(value))),
            None => Self::Digest(AuditHash::of_fields(fields(value))),
        }
    }

    /// Check `value` against the seal, under the scheme the reading side
    /// chose: with `key`, only a keyed seal is accepted, and without one only
    /// an unkeyed one.
    ///
    /// The scheme is never the seal's own to choose. A keyed seal is stored
    /// beside what it covers, so its carrier could swap it for an unkeyed
    /// digest of a value it changed; a reader holding a key refuses one.
    pub(crate) fn check<T: Sealable>(&self, value: &T, key: Option<&SealKey>) -> Result<()> {
        let matches = match (self, Self::new(value, key)) {
            (Self::Mac(found), Self::Mac(expected))
            | (Self::Digest(found), Self::Digest(expected)) => expected.matches(found),
            (Self::Digest(_), Self::Mac(_)) => {
                return Err(Error::integrity(format!(
                    "the {} has an unkeyed seal, and a keyed one is required",
                    T::NAME
                )));
            }
            (Self::Mac(_), Self::Digest(_)) => {
                return Err(Error::configuration(format!(
                    "the {} has a keyed seal, and no seal key is set to check it",
                    T::NAME
                )));
            }
        };
        if matches {
            Ok(())
        } else {
            Err(Error::integrity(format!(
                "the {} does not match its seal: it was changed after it was sealed",
                T::NAME
            )))
        }
    }
}

/// The fields a seal of `value` covers: its type's name, so no two kinds of
/// value seal alike, then the value's own.
pub(crate) fn fields<T: Sealable>(value: &T) -> Vec<Vec<u8>> {
    let mut fields = vec![T::NAME.as_bytes().to_vec()];
    fields.extend(value.seal_fields());
    fields
}

/// The seal fields of `parts`, each a part's path, its modality, and what the
/// seal covers of it: in path order, so the order the parts were held in does
/// not count, each part's path and fields counted, so no two lists of parts
/// encode alike.
pub(crate) fn part_fields<'a>(
    parts: impl IntoIterator<Item = (&'a PartId, &'static str, Vec<Vec<u8>>)>,
) -> Vec<Vec<u8>> {
    let mut parts: Vec<_> = parts.into_iter().collect();
    parts.sort_unstable_by_key(|(id, ..)| *id);
    let mut fields = Vec::new();
    for (id, modality, body) in parts {
        let segments: Vec<&str> = id.segments().collect();
        fields.push((segments.len() as u64).to_le_bytes().to_vec());
        fields.extend(segments.iter().map(|segment| segment.as_bytes().to_vec()));
        fields.push(modality.as_bytes().to_vec());
        fields.push((body.len() as u64).to_le_bytes().to_vec());
        fields.extend(body);
    }
    fields
}

/// A value a [`Seal`] covers, and a [`Restorer`](crate::Restorer) reads back:
/// a [`Report`], a [`Review`], or an [`ArtifactSet`].
///
/// Implemented only by those three.
///
/// [`Report`]: crate::Report
/// [`Review`]: crate::Review
/// [`ArtifactSet`]: crate::ArtifactSet
pub trait Sealable: private::Sealed {}

impl<T: private::Sealed> Sealable for T {}

pub(crate) mod private {
    use elide_core::Result;
    use serde::Deserializer;

    use super::super::Restorer;

    /// What a [`Sealable`](super::Sealable) provides: the fields its seal
    /// covers, and how a registry parses it. Implemented beside the
    /// registry's parsers.
    pub trait Sealed: Sized {
        /// What the value is called in an error.
        const NAME: &'static str;

        /// The fields the value's seal covers, in a canonical order.
        fn seal_fields(&self) -> Vec<Vec<u8>>;

        /// Parse the value through `registry`, without checking a seal.
        fn parse<'de, D: Deserializer<'de>>(registry: &Restorer, de: D) -> Result<Self>;
    }
}
