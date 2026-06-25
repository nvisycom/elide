//! [`Vault<K, V>`]: the pluggable token-vault contract a recoverable
//! operator resolves keys through.
//!
//! A vault is the out-of-band map a recoverable operator leans on: a
//! token replaces the original in the document, and the token resolves
//! back to its payload through the vault. A pseudonymizer is the same
//! shape — the same key resolves to the same generated replacement, so
//! every mention of one entity redacts consistently.
//!
//! The vault is generic over the key `K` as well as the value, so a
//! caller can key on a *full identity* (collision-free by construction)
//! rather than a lossy digest. The contract lives here; concrete backings
//! (the in-memory default, a KV store, a KMS-backed blob) live in the
//! toolkit crate.

use std::future::Future;

use crate::Result;

/// Token vault mapping keys of type `K` to cloneable values of type `V`.
///
/// Implementations must be safe to share across tasks and serve
/// concurrent reads/writes. Keys are whatever identity the caller chose
/// (an opaque token, or a structured tuple that equates exactly when two
/// inputs are the same); values are the payload the operator persists.
///
/// Generic over `K` so the caller controls identity: keying on a digest
/// trades space for collision risk, while keying on the full input keeps
/// equality exact. The async methods return `impl Future`, matching
/// [`Operator`]: a vault is a generic parameter (`V: Vault<K, …>`),
/// resolved and monomorphized at compile time rather than held behind a
/// trait object.
///
/// The write path is fused into [`get_or_try_insert_with`]: a value is
/// only ever stored as part of resolving a key, never blindly, so a
/// stored entry always wins and consistency holds by construction.
///
/// [`Operator`]: crate::operator::Operator
/// [`get_or_try_insert_with`]: Vault::get_or_try_insert_with
pub trait Vault<K, V: Clone + Send + Sync>: Send + Sync {
    /// Look up the value previously stored under `key`. Returns
    /// `Ok(None)` for unknown keys; reserve `Err` for backend failures.
    fn get(&self, key: &K) -> impl Future<Output = Result<Option<V>>> + Send;

    /// Return the value under `key`, or compute one with `init`, store it,
    /// and return that. The value already present always wins, so repeated
    /// first-sights of the same key resolve to a single value — the
    /// consistency guarantee a pseudonymizer relies on.
    ///
    /// `init` is synchronous and fallible: a [`Vault`] may run it under a
    /// lock to make the check-and-insert atomic, so it must not itself
    /// touch the vault or otherwise block. An [`Err`] from `init`
    /// propagates and stores nothing.
    fn get_or_try_insert_with<F>(&self, key: K, init: F) -> impl Future<Output = Result<V>> + Send
    where
        F: FnOnce() -> Result<V> + Send;
}
