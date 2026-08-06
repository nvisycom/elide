//! [`KeyProvider`]: supplies secret key material to the keyed operators
//! ([`HmacHash`], [`AesEncrypt`]) at apply time.
//!
//! [`HmacHash`]: crate::operators::HmacHash
//! [`AesEncrypt`]: crate::operators::AesEncrypt

use std::fmt;

use elide_core::entity::LabelRef;
use elide_core::Result;
use zeroize::Zeroizing;

/// Supplies secret key material at apply time, keyed by the entity's label.
///
/// The shared key-supply abstraction for the keyed operators — [`HmacHash`]
/// (keyed hash) and [`AesEncrypt`] (reversible encryption). The provider is
/// wired at operator construction from a secret store, an env var read at
/// startup, or a per-tenant KMS — never from serialized policy, so key
/// material never lives in a rule file. Keying by [`LabelRef`] lets one
/// operator draw a distinct key per label class (card numbers under one
/// key, SSNs under another).
///
/// The returned buffer is [`Zeroizing`], so it is wiped from memory on
/// drop. Length requirements are the operator's concern: HMAC accepts any
/// length, AES-256 requires exactly 32 bytes and errors otherwise.
///
/// [`HmacHash`]: crate::operators::HmacHash
/// [`AesEncrypt`]: crate::operators::AesEncrypt
pub trait KeyProvider: Send + Sync {
    /// The key for entities carrying `label`. The returned buffer is
    /// zeroized on drop.
    fn key(&self, label: &LabelRef) -> Result<Zeroizing<Vec<u8>>>;
}

/// A [`KeyProvider`] that returns one fixed key for every label.
///
/// The common case when a single deployment-wide key backs the whole
/// scheme. For per-label keys, implement [`KeyProvider`] directly.
pub struct StaticKey(Zeroizing<Vec<u8>>);

impl StaticKey {
    /// A provider returning `key` for every label.
    pub fn new(key: impl Into<Vec<u8>>) -> Self {
        Self(Zeroizing::new(key.into()))
    }
}

impl fmt::Debug for StaticKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print key material.
        f.debug_struct("StaticKey").finish_non_exhaustive()
    }
}

impl KeyProvider for StaticKey {
    fn key(&self, _label: &LabelRef) -> Result<Zeroizing<Vec<u8>>> {
        Ok(self.0.clone())
    }
}
