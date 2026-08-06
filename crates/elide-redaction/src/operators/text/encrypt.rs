//! [`AesEncrypt`]: reversibly replace an entity with an AES-256-GCM ciphertext.

use std::fmt;
use std::sync::Arc;

use aes_gcm::aead::{Aead, Generate, Nonce};
use aes_gcm::{Aes256Gcm, Key, KeyInit};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use elide_core::entity::{Entity, LabelRef};
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::operator::{LeakProfile, Operator, OperatorId, ReversibleOperator};
use elide_core::{Error, ErrorKind, Result};

use crate::operators::{KeyProvider, StaticKey};

/// Length of an AES-256 key, in bytes.
const KEY_LEN: usize = 32;

/// AES-GCM nonce length, in bytes (96 bits, the standard).
const NONCE_LEN: usize = 12;

/// Reversibly replace the matched value with an AES-256-GCM ciphertext.
///
/// The replacement is self-contained — base64 of `nonce ++ ciphertext` (the
/// authentication tag is part of the ciphertext) — so [`deanonymize`] needs
/// only the key, no out-of-band state. A fresh random nonce per call keeps
/// equal plaintexts from producing equal ciphertexts. The redaction is
/// recoverable by whoever holds the key, the basis for "redact for storage,
/// decrypt for authorized viewing" flows.
///
/// Key material comes from a [`KeyProvider`] wired at construction (from a
/// secret store, an env var read at startup, a per-tenant KMS), never from a
/// policy field, so it never lives in serialized rules. The provider is keyed
/// by [`LabelRef`], so a deployment can encrypt distinct label classes under
/// distinct keys; [`deanonymize`] resolves the same entity's label, so it
/// recovers under the same key. AES-256 needs a 32-byte key: a provider that
/// returns any other length is a [`Redaction`](ErrorKind::Redaction) error at
/// apply time, not a silent truncation.
///
/// [`deanonymize`]: ReversibleOperator::deanonymize
#[derive(Clone)]
pub struct AesEncrypt {
    keys: Arc<dyn KeyProvider>,
}

impl AesEncrypt {
    /// An encryptor drawing keys from `keys`.
    pub fn new(keys: Arc<dyn KeyProvider>) -> Self {
        Self { keys }
    }

    /// An encryptor backed by a single fixed 32-byte `key` for every label.
    ///
    /// The common single-key case; for per-label keys pass a custom
    /// [`KeyProvider`] to [`new`](Self::new). The length is validated at
    /// apply time, when the key is resolved.
    pub fn with_key(key: impl Into<Vec<u8>>) -> Self {
        Self::new(Arc::new(StaticKey::new(key)))
    }

    /// Identity shared by every modality's impl.
    fn id() -> OperatorId {
        OperatorId::new("encrypt", "1.0.0")
    }

    /// The cipher for `label`'s key, or a [`Redaction`](ErrorKind::Redaction)
    /// error if the provider returns a key that isn't 32 bytes.
    fn cipher(&self, label: &LabelRef) -> Result<Aes256Gcm> {
        let key = self.keys.key(label)?;
        let key: [u8; KEY_LEN] = key.as_slice().try_into().map_err(|_| {
            Error::new(
                ErrorKind::Redaction,
                "AES-256 requires a 32-byte key; the provider returned a different length",
            )
        })?;
        Ok(Aes256Gcm::new(&Key::<Aes256Gcm>::from(key)))
    }

    /// Encrypt `plaintext` to a base64 `nonce ++ ciphertext` blob under
    /// `label`'s key.
    fn encrypt_str(&self, label: &LabelRef, plaintext: &str) -> Result<String> {
        let nonce = Nonce::<Aes256Gcm>::generate();
        let ciphertext = self
            .cipher(label)?
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| Error::new(ErrorKind::Redaction, "encryption failed"))?;

        // Prepend the nonce so the replacement is self-describing.
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(BASE64.encode(blob))
    }

    /// Recover the plaintext from a text `replacement` this operator made
    /// under `label`'s key, or `None` if it isn't recoverable (not a
    /// substitution, not our blob, or the wrong key).
    fn decrypt_replacement(
        &self,
        label: &LabelRef,
        replacement: &TextReplacement,
    ) -> Result<Option<TextData>> {
        let TextReplacement::Substituted(encoded) = replacement else {
            // A `Removed` replacement carries nothing to recover.
            return Ok(None);
        };

        let Ok(blob) = BASE64.decode(encoded.as_bytes()) else {
            // Not one of our replacements (not valid base64).
            return Ok(None);
        };
        if blob.len() < NONCE_LEN {
            return Ok(None);
        }
        let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
        let Ok(nonce) = Nonce::<Aes256Gcm>::try_from(nonce) else {
            // Length was checked above, so this is unreachable in practice.
            return Ok(None);
        };

        match self.cipher(label)?.decrypt(&nonce, ciphertext) {
            // Authentication failed or wrong key: not recoverable here.
            Err(_) => Ok(None),
            Ok(plaintext) => {
                let text = String::from_utf8(plaintext).map_err(|_| {
                    Error::new(ErrorKind::Redaction, "decrypted bytes are not UTF-8")
                })?;
                Ok(Some(TextData::new(text)))
            }
        }
    }
}

impl fmt::Debug for AesEncrypt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print key material; the provider is opaque.
        f.debug_struct("AesEncrypt").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl Operator<Text> for AesEncrypt {
    fn id(&self) -> OperatorId {
        Self::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        // The original is recoverable given the key.
        LeakProfile::Recoverable
    }

    async fn anonymize(&self, entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(
            self.encrypt_str(&entity.label, data.as_str())?,
        ))
    }
}

#[async_trait::async_trait]
impl ReversibleOperator<Text> for AesEncrypt {
    async fn deanonymize(
        &self,
        entity: &Entity<Text>,
        replacement: &TextReplacement,
    ) -> Result<Option<TextData>> {
        self.decrypt_replacement(&entity.label, replacement)
    }
}

#[cfg(feature = "tabular")]
#[async_trait::async_trait]
impl Operator<Tabular> for AesEncrypt {
    fn id(&self) -> OperatorId {
        Self::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Recoverable
    }

    async fn anonymize(
        &self,
        entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TabularReplacement> {
        Ok(TextReplacement::substituted(self.encrypt_str(&entity.label, data.as_str())?).into())
    }
}

#[cfg(feature = "tabular")]
#[async_trait::async_trait]
impl ReversibleOperator<Tabular> for AesEncrypt {
    async fn deanonymize(
        &self,
        entity: &Entity<Tabular>,
        replacement: &TabularReplacement,
    ) -> Result<Option<TextData>> {
        // Only a cell treatment carries a recoverable ciphertext.
        match replacement {
            TabularReplacement::Cell(cell) => self.decrypt_replacement(&entity.label, cell),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::modality::text::TextLocation;
    use elide_core::primitive::Confidence;
    use zeroize::Zeroizing;

    use super::*;

    fn entity() -> Entity<Text> {
        let location = TextLocation::new(0, 5);
        let event = Event::pattern("t", Confidence::MAX, location.clone(), PatternEvent::default());
        Entity::new(
            LabelRef::new("email_address"),
            location,
            Confidence::MAX,
            Provenance::new(event),
        )
    }

    fn encryptor() -> AesEncrypt {
        AesEncrypt::with_key([7u8; 32].to_vec())
    }

    #[tokio::test]
    async fn round_trips_through_encrypt_then_decrypt() {
        let op = encryptor();
        let e = entity();

        let replacement = op
            .anonymize(&e, &TextData::new("alice@example.com"))
            .await
            .unwrap();
        let recovered = op.deanonymize(&e, &replacement).await.unwrap();
        assert_eq!(recovered, Some(TextData::new("alice@example.com")));
    }

    #[tokio::test]
    async fn equal_plaintexts_get_distinct_ciphertexts() {
        let op = encryptor();
        let e = entity();

        let a = op.anonymize(&e, &TextData::new("secret")).await.unwrap();
        let b = op.anonymize(&e, &TextData::new("secret")).await.unwrap();
        assert_ne!(a, b, "fresh nonce per call should differ");
    }

    #[tokio::test]
    async fn wrong_key_does_not_recover() {
        let e = entity();
        let replacement = encryptor()
            .anonymize(&e, &TextData::new("secret"))
            .await
            .unwrap();

        let other = AesEncrypt::with_key([9u8; 32].to_vec());
        assert_eq!(other.deanonymize(&e, &replacement).await.unwrap(), None);
    }

    #[tokio::test]
    async fn wrong_length_key_is_a_redaction_error() {
        // AES-256 needs exactly 32 bytes; a provider returning fewer must
        // error at apply time, not truncate or panic.
        let op = AesEncrypt::with_key(vec![0u8; 16]);
        let err = op
            .anonymize(&entity(), &TextData::new("secret"))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Redaction);
    }

    #[tokio::test]
    async fn per_label_provider_encrypts_under_distinct_keys() {
        // A per-label key means a ciphertext made under one label does not
        // decrypt when the entity carries another label.
        struct PerLabel;
        impl KeyProvider for PerLabel {
            fn key(&self, label: &LabelRef) -> Result<Zeroizing<Vec<u8>>> {
                // Derive a deterministic 32-byte key from the label.
                let mut key = vec![0u8; 32];
                for (i, b) in label.as_str().bytes().enumerate() {
                    key[i % 32] ^= b;
                }
                Ok(Zeroizing::new(key))
            }
        }
        let op = AesEncrypt::new(Arc::new(PerLabel));

        let card = {
            let loc = TextLocation::new(0, 5);
            let event = Event::pattern("t", Confidence::MAX, loc.clone(), PatternEvent::default());
            Entity::<Text>::new(LabelRef::new("payment_card"), loc, Confidence::MAX, Provenance::new(event))
        };
        let ssn = {
            let loc = TextLocation::new(0, 5);
            let event = Event::pattern("t", Confidence::MAX, loc.clone(), PatternEvent::default());
            Entity::<Text>::new(LabelRef::new("ssn"), loc, Confidence::MAX, Provenance::new(event))
        };

        let replacement = op.anonymize(&card, &TextData::new("secret")).await.unwrap();
        // Same ciphertext, decrypted under the wrong label's key: not recovered.
        assert_eq!(op.deanonymize(&ssn, &replacement).await.unwrap(), None);
        // Under the right label: recovered.
        assert_eq!(
            op.deanonymize(&card, &replacement).await.unwrap(),
            Some(TextData::new("secret"))
        );
    }
}
