//! The `Hash` operator: replace the matched value with a one-way SHA-2
//! hash.

use elide_core::Result;
use elide_core::entity::Entity;
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::redaction::{LeakProfile, Operator, OperatorId};
use sha2::{Digest, Sha256, Sha512};

/// Which SHA-2 variant to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HashAlgorithm {
    /// SHA-256 — 32-byte digest, 64-char hex.
    #[default]
    Sha256,
    /// SHA-512 — 64-byte digest, 128-char hex.
    Sha512,
}

/// One-way SHA-2 hash operator.
///
/// Replaces the value with the lowercase hex of its digest. An optional
/// salt is prepended before hashing, which blocks precomputed rainbow
/// attacks and makes equal plaintext hash differently across
/// deployments.
#[derive(Debug, Clone, Default)]
pub struct Hash {
    algorithm: HashAlgorithm,
    salt: Vec<u8>,
}

impl Hash {
    /// A hash operator using `algorithm`, with no salt.
    pub fn new(algorithm: HashAlgorithm) -> Self {
        Self {
            algorithm,
            salt: Vec::new(),
        }
    }

    /// SHA-256, no salt — the common default.
    pub fn sha256() -> Self {
        Self::new(HashAlgorithm::Sha256)
    }

    /// SHA-512, no salt.
    pub fn sha512() -> Self {
        Self::new(HashAlgorithm::Sha512)
    }

    /// Attach a salt prepended to the value before hashing.
    #[must_use]
    pub fn with_salt(mut self, salt: impl Into<Vec<u8>>) -> Self {
        self.salt = salt.into();
        self
    }

    /// The lowercase hex digest of `value` under this operator.
    fn digest(&self, value: &str) -> String {
        match self.algorithm {
            HashAlgorithm::Sha256 => hex::encode(
                Sha256::new()
                    .chain_update(&self.salt)
                    .chain_update(value.as_bytes())
                    .finalize(),
            ),
            HashAlgorithm::Sha512 => hex::encode(
                Sha512::new()
                    .chain_update(&self.salt)
                    .chain_update(value.as_bytes())
                    .finalize(),
            ),
        }
    }
}

impl Operator<Text> for Hash {
    fn id(&self) -> OperatorId {
        OperatorId::new("hash", "1.0.0")
    }

    fn leak_profile(&self) -> LeakProfile {
        // Recoverable when the candidate plaintext space is small enough
        // to brute-force; salting raises the bar but doesn't remove it.
        LeakProfile::Recoverable
    }

    async fn anonymize(&self, _entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(self.digest(data.as_str())))
    }
}
