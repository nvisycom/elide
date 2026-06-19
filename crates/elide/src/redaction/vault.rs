//! [`InMemoryVault`]: the batteries-included [`Vault`] backing.
//!
//! [`Vault`]: elide_core::redaction::Vault

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use elide_core::Result;
use elide_core::redaction::Vault;

/// Process-local [`Vault`] backed by a locked [`HashMap`].
///
/// The default vault: holds everything in memory behind a [`Mutex`], so
/// it is shareable and concurrency-safe but not durable — the contents
/// vanish when the last handle is dropped. Suited to a single
/// anonymization run or to tests; swap in a durable [`Vault`] for
/// cross-run consistency.
///
/// The map sits behind an internal [`Arc`], so [`Clone`] is cheap and
/// every clone shares *one* underlying map. That is what lets a single
/// vault back several rules (or several cloned pseudonymizing operators)
/// while keeping their surrogates consistent — cloning hands out another
/// handle, never a separate copy.
///
/// [`Vault`]: elide_core::redaction::Vault
#[derive(Debug, Default, Clone)]
pub struct InMemoryVault<K, V> {
    entries: Arc<Mutex<HashMap<K, V>>>,
}

impl<K, V> InMemoryVault<K, V> {
    /// An empty in-memory vault.
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<K, V> Vault<K, V> for InMemoryVault<K, V>
where
    K: Eq + Hash + Send + Sync,
    V: Clone + Send + Sync,
{
    async fn get(&self, key: &K) -> Result<Option<V>> {
        Ok(self
            .entries
            .lock()
            .expect("vault mutex poisoned")
            .get(key)
            .cloned())
    }

    async fn get_or_try_insert_with<F>(&self, key: K, init: F) -> Result<V>
    where
        F: FnOnce() -> Result<V> + Send,
    {
        // The whole check-and-insert runs under one lock and never crosses
        // an await, so it is atomic: a concurrent first-sight of the same
        // key sees the value the first caller inserted.
        let mut entries = self.entries.lock().expect("vault mutex poisoned");
        if let Some(existing) = entries.get(&key) {
            return Ok(existing.clone());
        }
        let value = init()?;
        entries.insert(key, value.clone());
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_then_get_round_trips() {
        let vault = InMemoryVault::<&str, u32>::new();
        assert!(vault.get(&"missing").await.unwrap().is_none());

        let inserted = vault.get_or_try_insert_with("k", || Ok(7)).await.unwrap();
        assert_eq!(inserted, 7);
        assert_eq!(vault.get(&"k").await.unwrap(), Some(7));
    }

    #[tokio::test]
    async fn get_or_try_insert_with_keeps_the_first_value() {
        let vault = InMemoryVault::<&str, u32>::new();

        let first = vault.get_or_try_insert_with("k", || Ok(1)).await.unwrap();
        // Second call finds the key present and never runs init.
        let second = vault
            .get_or_try_insert_with("k", || panic!("init must not run on a hit"))
            .await
            .unwrap();

        assert_eq!(first, 1);
        assert_eq!(second, 1);
        assert_eq!(vault.get(&"k").await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn get_or_try_insert_with_propagates_init_error() {
        use elide_core::{Error, ErrorKind};
        let vault = InMemoryVault::<&str, u32>::new();

        let result = vault
            .get_or_try_insert_with("k", || Err(Error::new(ErrorKind::Redaction, "boom")))
            .await;

        assert!(result.is_err());
        // A failed init stores nothing.
        assert!(vault.get(&"k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn distinct_keys_do_not_collide() {
        // A structured key: equal only when every field matches.
        let vault = InMemoryVault::<(&str, u32), &str>::new();
        vault
            .get_or_try_insert_with(("a", 1), || Ok("first"))
            .await
            .unwrap();
        vault
            .get_or_try_insert_with(("a", 2), || Ok("second"))
            .await
            .unwrap();
        assert_eq!(vault.get(&("a", 1)).await.unwrap(), Some("first"));
        assert_eq!(vault.get(&("a", 2)).await.unwrap(), Some("second"));
    }
}
