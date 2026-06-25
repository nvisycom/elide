//! [`Encrypt`]: reversibly replace an entity with an AES-256-GCM ciphertext.

use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use elide_core::entity::Entity;
#[cfg(feature = "tabular")]
use elide_core::modality::tabular::{Tabular, TabularReplacement};
use elide_core::modality::text::{Text, TextData, TextReplacement};
use elide_core::operator::{LeakProfile, Operator, OperatorId, ReversibleOperator};
use elide_core::{Error, ErrorKind, Result};

use crate::redaction::key_provider::KeyProvider;

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
/// The key comes from a [`KeyProvider`] rather than the policy, so key
/// material never lives in serialized rules.
///
/// [`deanonymize`]: ReversibleOperator::deanonymize
#[derive(Debug, Clone)]
pub struct Encrypt<K> {
    keys: K,
}

impl<K: KeyProvider> Encrypt<K> {
    /// An encryptor drawing its key from `keys`.
    pub fn new(keys: K) -> Self {
        Self { keys }
    }

    /// Identity shared by every modality's impl.
    fn id() -> OperatorId {
        OperatorId::new("encrypt", "1.0.0")
    }

    /// The cipher bound to the current key.
    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(self.keys.key().into())
    }

    /// Encrypt `plaintext` to a base64 `nonce ++ ciphertext` blob.
    fn encrypt_str(&self, plaintext: &str) -> Result<String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher()
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| Error::new(ErrorKind::Validation, "encryption failed"))?;

        // Prepend the nonce so the replacement is self-describing.
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(BASE64.encode(blob))
    }

    /// Recover the plaintext from a text `replacement` this operator made,
    /// or `None` if it isn't recoverable (not a substitution, not our blob,
    /// or the wrong key).
    fn decrypt_replacement(&self, replacement: &TextReplacement) -> Result<Option<TextData>> {
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

        match self.cipher().decrypt(Nonce::from_slice(nonce), ciphertext) {
            // Authentication failed or wrong key: not recoverable here.
            Err(_) => Ok(None),
            Ok(plaintext) => {
                let text = String::from_utf8(plaintext).map_err(|_| {
                    Error::new(ErrorKind::Validation, "decrypted bytes are not UTF-8")
                })?;
                Ok(Some(TextData::new(text)))
            }
        }
    }
}

impl<K: KeyProvider> Operator<Text> for Encrypt<K> {
    fn id(&self) -> OperatorId {
        Self::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        // The original is recoverable given the key.
        LeakProfile::Recoverable
    }

    async fn anonymize(&self, _entity: &Entity<Text>, data: &TextData) -> Result<TextReplacement> {
        Ok(TextReplacement::substituted(
            self.encrypt_str(data.as_str())?,
        ))
    }
}

impl<K: KeyProvider> ReversibleOperator<Text> for Encrypt<K> {
    async fn deanonymize(
        &self,
        _entity: &Entity<Text>,
        replacement: &TextReplacement,
    ) -> Result<Option<TextData>> {
        self.decrypt_replacement(replacement)
    }
}

#[cfg(feature = "tabular")]
impl<K: KeyProvider> Operator<Tabular> for Encrypt<K> {
    fn id(&self) -> OperatorId {
        Self::id()
    }

    fn leak_profile(&self) -> LeakProfile {
        LeakProfile::Recoverable
    }

    async fn anonymize(
        &self,
        _entity: &Entity<Tabular>,
        data: &TextData,
    ) -> Result<TabularReplacement> {
        Ok(TextReplacement::substituted(self.encrypt_str(data.as_str())?).into())
    }
}

#[cfg(feature = "tabular")]
impl<K: KeyProvider> ReversibleOperator<Tabular> for Encrypt<K> {
    async fn deanonymize(
        &self,
        _entity: &Entity<Tabular>,
        replacement: &TabularReplacement,
    ) -> Result<Option<TextData>> {
        // Only a cell treatment carries a recoverable ciphertext.
        match replacement {
            TabularReplacement::Cell(cell) => self.decrypt_replacement(cell),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::provenance::{Event, PatternEvent, Provenance};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::*;
    use crate::redaction::key_provider::StaticKey;

    fn entity() -> Entity<Text> {
        let location = TextLocation::new(0, 5);
        let event = Event::pattern(
            "t",
            Confidence::MAX,
            location.clone(),
            PatternEvent::default(),
        );
        Entity::new(
            LabelRef::new("EMAIL_ADDRESS"),
            location,
            Confidence::MAX,
            Provenance::new(event),
        )
    }

    fn encryptor() -> Encrypt<StaticKey> {
        Encrypt::new(StaticKey::new([7u8; 32]))
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

        let other = Encrypt::new(StaticKey::new([9u8; 32]));
        assert_eq!(other.deanonymize(&e, &replacement).await.unwrap(), None);
    }
}
