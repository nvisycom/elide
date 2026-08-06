//! [`Sha2Algorithm`]: the SHA-2 digest width shared by the hashing
//! operators.

/// Which SHA-2 variant a hashing operator uses.
///
/// Shared by [`Sha2Hash`] (unkeyed digest) and [`HmacHash`] (keyed HMAC):
/// both pick the same underlying width, so the choice lives in one enum
/// rather than one per operator.
///
/// [`Sha2Hash`]: super::Sha2Hash
/// [`HmacHash`]: super::HmacHash
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Sha2Algorithm {
    /// SHA-256 — 32-byte digest, 64-char hex.
    #[default]
    Sha256,
    /// SHA-512 — 64-byte digest, 128-char hex.
    Sha512,
}
