//! [`StaticKey`]: a [`KeyProvider`] wrapping one fixed key.

use super::{Key, KeyProvider};

/// A [`KeyProvider`] holding one fixed key for the process lifetime.
///
/// The simplest provider: hand it a 256-bit key obtained out-of-band (an
/// env var, a secret-store fetch at startup). For key rotation or per-tenant
/// keys, implement [`KeyProvider`] directly instead.
#[derive(Clone)]
pub struct StaticKey {
    key: Key,
}

impl StaticKey {
    /// Wrap a 256-bit key.
    pub fn new(key: Key) -> Self {
        Self { key }
    }
}

impl KeyProvider for StaticKey {
    fn key(&self) -> &Key {
        &self.key
    }
}

impl std::fmt::Debug for StaticKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material.
        f.debug_struct("StaticKey").finish_non_exhaustive()
    }
}
