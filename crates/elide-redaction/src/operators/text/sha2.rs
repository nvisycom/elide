//! The [`Sha2Hash`] operator: replace the matched value with a one-way
//! SHA-2 hash.

use bytes::Bytes;
use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::TextRedactable;
use elide_core::modality::text::TextReplacement;
use elide_core::primitive::ComponentId;
use elide_core::redaction::{LeakProfile, Operator};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};

use crate::operators::Sha2Algorithm;

/// Replaces an entity's text with its one-way SHA-2 hash, in hex.
///
/// Replaces the value with the lowercase hex of its digest. An optional
/// salt is prepended before hashing, which blocks precomputed rainbow
/// attacks and makes equal plaintext hash differently across
/// deployments.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Sha2Hash {
    algorithm: Sha2Algorithm,
    salt: Bytes,
}

impl Sha2Hash {
    /// A hash operator using `algorithm`, with no salt.
    pub fn new(algorithm: Sha2Algorithm) -> Self {
        Self {
            algorithm,
            salt: Bytes::new(),
        }
    }

    /// SHA-256, no salt, the common default.
    pub fn sha256() -> Self {
        Self::new(Sha2Algorithm::Sha256)
    }

    /// SHA-512, no salt.
    pub fn sha512() -> Self {
        Self::new(Sha2Algorithm::Sha512)
    }

    /// Attach a salt prepended to the value before hashing.
    #[must_use]
    pub fn with_salt(mut self, salt: impl Into<Bytes>) -> Self {
        self.salt = salt.into();
        self
    }

    /// The lowercase hex digest of `value` under this operator.
    fn digest(&self, value: &str) -> String {
        match self.algorithm {
            Sha2Algorithm::Sha256 => hex::encode(
                Sha256::new()
                    .chain_update(&self.salt)
                    .chain_update(value.as_bytes())
                    .finalize(),
            ),
            Sha2Algorithm::Sha512 => hex::encode(
                Sha512::new()
                    .chain_update(&self.salt)
                    .chain_update(value.as_bytes())
                    .finalize(),
            ),
        }
    }
}

#[async_trait::async_trait]
impl<M: TextRedactable> Operator<M> for Sha2Hash {
    fn id(&self) -> ComponentId {
        ComponentId::new("hash", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // Recoverable when the candidate plaintext space is small enough
        // to brute-force; salting raises the bar but doesn't remove it.
        LeakProfile::Recoverable
    }

    async fn anonymize(&self, _entity: &Entity<M>, data: &M::Data) -> Result<M::Replacement> {
        let digest = self.digest(data.as_ref());
        Ok(M::Replacement::from(TextReplacement::substituted(digest)))
    }
}
