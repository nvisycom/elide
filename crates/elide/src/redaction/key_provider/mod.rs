//! [`KeyProvider`]: supplies the symmetric key the [`Encrypt`] operator
//! uses, kept out of policy config so keys never live in serialized rules.
//!
//! [`Encrypt`]: crate::redaction::operators::Encrypt

mod static_key;

pub use self::static_key::StaticKey;

/// Length of an AES-256 key, in bytes.
pub const KEY_LEN: usize = 32;

/// An AES-256 key.
pub type Key = [u8; KEY_LEN];

/// Supplies the symmetric key for encryption and decryption.
///
/// A seam so the key can come from anywhere — an env var, a KMS, an HSM —
/// without policy code (which may be serialized from TOML) ever holding the
/// key material. The simplest implementation, [`StaticKey`], wraps a fixed
/// key; production deployments implement this over their secret store.
pub trait KeyProvider: Send + Sync {
    /// The key to encrypt with and decrypt against.
    fn key(&self) -> &Key;
}
