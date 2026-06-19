//! [`Store<K, V>`]: pluggable token vault mapping keys to cloneable
//! values, plus a default [`InMemoryStore`] backing.
//!
//! A vault is the out-of-band map a recoverable operator leans on: a
//! token replaces the original in the document, and the token resolves
//! back to its payload through the vault. Memoization is the same shape
//! seen from the other side — the same input keys the same entry, so an
//! inner operator runs only once per distinct payload.
//!
//! The store is generic over the key `K` as well as the value, so a
//! caller can key on a *full identity* (collision-free by construction)
//! rather than a lossy digest. Implementations pick their own backing
//! (in-memory map, KV store, KMS-backed encrypted blob) and are chosen at
//! compile time. [`InMemoryStore`] is the batteries-included one — a
//! locked map, process-local, gone when dropped.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::Mutex;

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
/// [`Operator`]: a vault is a generic parameter (`S: Store<K, V>`),
/// resolved and monomorphized at compile time rather than held behind a
/// trait object.
///
/// [`Operator`]: crate::redaction::Operator
pub trait Store<K, V: Clone + Send + Sync>: Send + Sync {
    /// Persist `value` under `key`. Re-using a key replaces the prior
    /// value.
    fn put(&self, key: K, value: V) -> impl Future<Output = Result<()>> + Send;

    /// Look up the value previously stored under `key`. Returns
    /// `Ok(None)` for unknown keys; reserve `Err` for backend failures.
    fn get(&self, key: &K) -> impl Future<Output = Result<Option<V>>> + Send;
}

/// Process-local [`Store`] backed by a locked [`HashMap`].
///
/// The default vault: holds everything in memory behind a [`Mutex`], so
/// it is shareable and concurrency-safe but not durable — the contents
/// vanish when the store is dropped. Suited to a single anonymization run
/// or to tests; swap in a durable [`Store`] for cross-run consistency.
#[derive(Debug, Default)]
pub struct InMemoryStore<K, V> {
    entries: Mutex<HashMap<K, V>>,
}

impl<K, V> InMemoryStore<K, V> {
    /// An empty in-memory store.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl<K, V> Store<K, V> for InMemoryStore<K, V>
where
    K: Eq + Hash + Send + Sync,
    V: Clone + Send + Sync,
{
    async fn put(&self, key: K, value: V) -> Result<()> {
        // The guard never crosses an await, so a std Mutex is enough and
        // the returned future stays Send.
        self.entries
            .lock()
            .expect("vault mutex poisoned")
            .insert(key, value);
        Ok(())
    }

    async fn get(&self, key: &K) -> Result<Option<V>> {
        Ok(self
            .entries
            .lock()
            .expect("vault mutex poisoned")
            .get(key)
            .cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let store = InMemoryStore::<String, String>::new();
        assert!(store.get(&"missing".to_owned()).await.unwrap().is_none());

        store.put("k".to_owned(), "v".to_owned()).await.unwrap();
        assert_eq!(
            store.get(&"k".to_owned()).await.unwrap(),
            Some("v".to_owned())
        );
    }

    #[tokio::test]
    async fn put_overwrites_existing_key() {
        let store = InMemoryStore::<&str, u32>::new();
        store.put("k", 1).await.unwrap();
        store.put("k", 2).await.unwrap();
        assert_eq!(store.get(&"k").await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn distinct_keys_do_not_collide() {
        // A structured key: equal only when every field matches.
        let store = InMemoryStore::<(&str, u32), &str>::new();
        store.put(("a", 1), "first").await.unwrap();
        store.put(("a", 2), "second").await.unwrap();
        assert_eq!(store.get(&("a", 1)).await.unwrap(), Some("first"));
        assert_eq!(store.get(&("a", 2)).await.unwrap(), Some("second"));
    }
}
