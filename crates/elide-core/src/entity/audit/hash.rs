//! [`AuditHash`]: a BLAKE3 digest, the link in an entity's audit DAG.

use std::fmt;
use std::hash::Hash;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A 32-byte BLAKE3 digest.
///
/// The tamper-evident link in an [`AuditLog`] DAG: each [`AuditEvent`] hashes
/// its payload together with its parents' hashes, so altering any event breaks
/// every event downstream of it. Displays and (with the `serde` feature)
/// (de)serializes as lowercase hex.
///
/// The all-zero hash is [`GENESIS`](AuditHash::GENESIS), the link a birth event
/// (one with no parents) chains from.
///
/// [`AuditLog`]: crate::entity::audit::AuditLog
/// [`AuditEvent`]: crate::entity::audit::AuditEvent
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(transparent))]
#[cfg_attr(
    feature = "schema",
    derive(schemars::JsonSchema),
    schemars(transparent)
)]
pub struct AuditHash(#[cfg_attr(feature = "schema", schemars(with = "String"))] HexBytes);

impl AuditHash {
    /// The genesis link: the all-zero hash a birth event chains from.
    pub const GENESIS: Self = Self(HexBytes([0u8; 32]));

    /// The raw 32 digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0.0
    }
}

/// Streaming builder for an [`AuditHash`]: audit payloads fold their fields in
/// through its methods, and [`finish`](AuditHasher::finish) yields the digest.
///
/// Feeds a BLAKE3 hasher directly, no intermediate buffer is allocated.
/// [`bytes`](Self::bytes) / [`opt`](Self::opt) length-prefix each field so
/// concatenated fields cannot be confused across a boundary (`"ab" + "c"`
/// differs from `"a" + "bc"`); [`raw`](Self::raw) / [`byte`](Self::byte) write
/// fixed-width spine values that need no delimiter.
///
/// [`finish`]: AuditHasher::finish
pub(crate) struct AuditHasher(blake3::Hasher);

impl AuditHasher {
    /// A fresh hasher with nothing folded in yet.
    pub(crate) fn new() -> Self {
        Self(blake3::Hasher::new())
    }

    /// Fold in raw bytes with no length prefix, for fixed-width spine values
    /// (a hash, a little-endian integer) whose width is known, so no delimiter
    /// is needed.
    pub(crate) fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.update(bytes);
        self
    }

    /// Fold in a single byte (a discriminant, a presence flag, a bool).
    pub(crate) fn byte(&mut self, byte: u8) -> &mut Self {
        self.0.update(&[byte]);
        self
    }

    /// Fold in a length-prefixed byte string, so a field boundary is
    /// unambiguous.
    pub(crate) fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.raw(&(bytes.len() as u64).to_le_bytes());
        self.raw(bytes)
    }

    /// Fold in an optional byte string: a presence byte, then the value if
    /// present.
    pub(crate) fn opt(&mut self, value: Option<&[u8]>) -> &mut Self {
        match value {
            Some(bytes) => {
                self.byte(1);
                self.bytes(bytes)
            }
            None => self.byte(0),
        }
    }

    /// Finish hashing and return the digest.
    pub(crate) fn finish(&self) -> AuditHash {
        AuditHash(HexBytes(*self.0.finalize().as_bytes()))
    }
}

impl fmt::Debug for AuditHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AuditHash({self})")
    }
}

impl fmt::Display for AuditHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// The 32 digest bytes, serialized as a lowercase-hex string so a serialized
/// trail is human-readable and diff-friendly.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct HexBytes([u8; 32]);

#[cfg(feature = "serde")]
impl Serialize for HexBytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for HexBytes {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let hex = String::deserialize(deserializer)?;
        let bytes = hex::decode(&hex).map_err(D::Error::custom)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| D::Error::custom("audit hash must be 32 bytes"))?;
        Ok(Self(bytes))
    }
}
