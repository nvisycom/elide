//! [`HmacHash`]: replace the matched value with a keyed HMAC digest.
//! Distinct from [`Sha2Hash`], whose salt is not a secret key.
//!
//! [`Sha2Hash`]: crate::operators::Sha2Hash

use std::fmt;
use std::sync::Arc;

use elide_core::entity::{Entity, LabelRef};
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::operator::{LeakProfile, Operator, OperatorId};
use elide_core::Result;
use hmac::{Hmac, KeyInit, Mac};
#[cfg(feature = "serde")]
use serde::Serialize;
use sha2::{Sha256, Sha512};

use crate::operators::{KeyProvider, Sha2Algorithm, StaticKey};

/// Keyed one-way HMAC-SHA-2 hash operator.
///
/// Replaces the value with the lowercase hex of `HMAC-SHA-N(key, value)`.
/// Unlike [`Sha2Hash`]'s salt — which is public and only blocks precomputed
/// rainbow tables — the HMAC key stays secret, so an attacker who obtains
/// the ciphertext database still cannot enumerate a small input space
/// without it. This is the "keyed cryptographic hash" PCI DSS v4.0.1
/// §3.5.1 lists as a permitted method for rendering a stored PAN
/// unreadable.
///
/// The digest is deterministic for a given key, so equal plaintexts
/// tokenize to equal digests — the property that makes it a stable token.
/// Key material comes from a [`KeyProvider`] wired at construction, never
/// from serialized policy.
///
/// [`Sha2Hash`]: crate::operators::Sha2Hash
/// Only the policy config (`algorithm`) serializes; the [`KeyProvider`] is
/// skipped — key material is never part of serialized policy and is re-wired
/// at construction when a selection is rebuilt. `Serialize` only: the skipped
/// provider has no default to deserialize back into.
#[derive(Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HmacHash {
    algorithm: Sha2Algorithm,
    #[cfg_attr(feature = "serde", serde(skip))]
    #[cfg_attr(feature = "schema", schemars(skip))]
    keys: Arc<dyn KeyProvider>,
}

impl HmacHash {
    /// Identity shared by every modality's impl.
    fn id() -> OperatorId {
        OperatorId::new("hmac", "1.0.0")
    }

    /// An HMAC operator over `algorithm`, drawing keys from `keys`.
    pub fn new(algorithm: Sha2Algorithm, keys: Arc<dyn KeyProvider>) -> Self {
        Self { algorithm, keys }
    }

    /// HMAC-SHA-256 backed by a single fixed `key` for every label.
    pub fn sha256(key: impl Into<Vec<u8>>) -> Self {
        Self::new(Sha2Algorithm::Sha256, Arc::new(StaticKey::new(key)))
    }

    /// HMAC-SHA-512 backed by a single fixed `key` for every label.
    pub fn sha512(key: impl Into<Vec<u8>>) -> Self {
        Self::new(Sha2Algorithm::Sha512, Arc::new(StaticKey::new(key)))
    }

    /// The lowercase hex HMAC digest of `value` under the key for `label`.
    fn digest(&self, label: &LabelRef, value: &str) -> Result<String> {
        let key = self.keys.key(label)?;
        // HMAC accepts a key of any length, so `new_from_slice` is infallible
        // here; treat the documented error as unreachable rather than swallow.
        Ok(match self.algorithm {
            Sha2Algorithm::Sha256 => {
                let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key)
                    .expect("HMAC accepts keys of any length");
                mac.update(value.as_bytes());
                hex::encode(mac.finalize().into_bytes())
            }
            Sha2Algorithm::Sha512 => {
                let mut mac = <Hmac<Sha512> as KeyInit>::new_from_slice(&key)
                    .expect("HMAC accepts keys of any length");
                mac.update(value.as_bytes());
                hex::encode(mac.finalize().into_bytes())
            }
        })
    }
}

impl fmt::Debug for HmacHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print key material; the provider is opaque.
        f.debug_struct("HmacHash")
            .field("algorithm", &self.algorithm)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Operator<Text> for HmacHash {
    fn id(&self) -> OperatorId {
        HmacHash::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        // Recoverable when the candidate plaintext space is small enough to
        // brute-force *and* the attacker has the key; the key secrecy
        // raises the bar over a public salt but doesn't change the profile.
        LeakProfile::Recoverable
    }

    async fn anonymize(&self, entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(
            self.digest(&entity.label, data.as_str())?,
        ))
    }
}

#[cfg(feature = "tabular")]
#[async_trait::async_trait]
impl Operator<Tabular> for HmacHash {
    fn id(&self) -> OperatorId {
        HmacHash::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Recoverable
    }

    async fn anonymize(
        &self,
        entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TabularReplacement> {
        Ok(TextReplacement::substituted(self.digest(&entity.label, data.as_str())?).into())
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::modality::text::TextLocation;
    use elide_core::primitive::Confidence;
    use zeroize::Zeroizing;

    use super::*;

    fn entity(label: &str) -> Entity<Text> {
        let location = TextLocation::new(0, 5);
        let event = Event::pattern("t", Confidence::MAX, location.clone(), PatternEvent::default());
        Entity::new(
            LabelRef::new(label),
            location,
            Confidence::MAX,
            Provenance::new(event),
        )
    }

    #[tokio::test]
    async fn digest_is_stable_for_a_fixed_key() {
        let op = HmacHash::sha256(b"secret-key".to_vec());
        let e = entity("payment_card");
        let a = op.anonymize(&e, &TextData::new("4111111111111234")).await.unwrap();
        let b = op.anonymize(&e, &TextData::new("4111111111111234")).await.unwrap();
        assert_eq!(a, b, "same key + value must tokenize identically");
    }

    #[tokio::test]
    async fn digest_changes_with_the_key() {
        let e = entity("payment_card");
        let value = TextData::new("4111111111111234");
        let a = HmacHash::sha256(b"key-a".to_vec()).anonymize(&e, &value).await.unwrap();
        let b = HmacHash::sha256(b"key-b".to_vec()).anonymize(&e, &value).await.unwrap();
        assert_ne!(a, b, "a different key must yield a different digest");
    }

    #[tokio::test]
    async fn sha256_and_sha512_select_distinct_algorithms() {
        // Our dispatch must route each constructor to its own variant — a
        // swap bug would make both digests identical. (The digest *widths*
        // are a property of the sha2 crate, so we don't assert them here.)
        let e = entity("payment_card");
        let value = TextData::new("4111111111111234");
        let s256 = HmacHash::sha256(b"k".to_vec()).anonymize(&e, &value).await.unwrap();
        let s512 = HmacHash::sha512(b"k".to_vec()).anonymize(&e, &value).await.unwrap();
        assert_ne!(s256, s512, "the two algorithms must not collide");
    }

    #[tokio::test]
    async fn per_label_provider_keys_by_label() {
        struct PerLabel;
        impl KeyProvider for PerLabel {
            fn key(&self, label: &LabelRef) -> Result<Zeroizing<Vec<u8>>> {
                Ok(Zeroizing::new(label.as_str().as_bytes().to_vec()))
            }
        }
        let op = HmacHash::new(Sha2Algorithm::Sha256, Arc::new(PerLabel));
        let value = TextData::new("same-value");
        let a = op.anonymize(&entity("payment_card"), &value).await.unwrap();
        let b = op.anonymize(&entity("ssn"), &value).await.unwrap();
        assert_ne!(a, b, "distinct labels draw distinct keys, so distinct digests");
    }
}
