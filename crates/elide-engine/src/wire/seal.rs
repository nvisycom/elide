//! [`Seal`], [`SealKey`], and [`Sealable`]: what a report, a review, or an
//! artifact set must match to be read back.

use std::fmt;

use elide_core::entity::audit::AuditHash;
use elide_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::PartId;

/// A secret key for keyed seals.
///
/// With it, [`Seal::keyed`] computes a message authentication code: a seal
/// that only a holder of the key can produce, so it can be stored beside what
/// it covers. The side that reads a keyed seal back holds the key too, set with
/// [`Restorer::with_seal_key`](crate::Restorer::with_seal_key) or
/// [`Orchestrator::with_seal_key`](crate::Orchestrator::with_seal_key). Keep it
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
/// `{ "mac": hex }`:
///
/// - A [`Digest`](Self::Digest), from [`Seal::of`], is unkeyed: anyone can
///   compute one, so it protects only if it is stored where the carrier cannot
///   write: a signed log, a write-once bucket, a timestamping service.
/// - A [`Mac`](Self::Mac), from [`Seal::keyed`], needs the [`SealKey`] to
///   compute, so it can be stored beside what it covers, as long as the carrier
///   does not hold the key.
///
/// Read a value back against its seal with [`Restorer::restore`](crate::Restorer::restore).
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
    /// The unkeyed seal of `value`.
    #[must_use]
    pub fn of(value: &impl Sealable) -> Self {
        Self::Digest(AuditHash::of_fields(value.seal_fields()))
    }

    /// The seal of `value` keyed with `key`.
    #[must_use]
    pub fn keyed(value: &impl Sealable, key: &SealKey) -> Self {
        Self::Mac(AuditHash::of_fields_keyed(&key.0, value.seal_fields()))
    }

    /// Check `value` against the seal, with `key` for a keyed one.
    pub(crate) fn check<T: Sealable>(&self, value: &T, key: Option<&SealKey>) -> Result<()> {
        let matches = match self {
            Self::Digest(digest) => AuditHash::of_fields(value.seal_fields()).matches(digest),
            Self::Mac(mac) => {
                let key = key.ok_or_else(|| {
                    Error::configuration(format!(
                        "the {} has a keyed seal, and no seal key is set to check it",
                        T::NAME
                    ))
                })?;
                AuditHash::of_fields_keyed(&key.0, value.seal_fields()).matches(mac)
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
