//! In-process implementation of [`StateStore`](super::StateStore).

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::state::{Checkpoint, ResumeKey, StateStore, StoreError};

/// In-process `StateStore` backed by a `HashMap` behind a tokio `RwLock`.
///
/// `Clone` is cheap (an `Arc` bump); two cloned handles share the same
/// underlying map. State is lost when the last handle is dropped.
///
/// For state that survives process restarts, use
/// [`JsonFileStore`](super::JsonFileStore).
#[derive(Debug, Default, Clone)]
pub struct MemoryStore {
    inner: Arc<RwLock<HashMap<ResumeKey, Checkpoint>>>,
}

impl MemoryStore {
    /// Create an empty `MemoryStore`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl StateStore for MemoryStore {
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, StoreError> {
        let map = self.inner.read().await;
        Ok(map.get(key).cloned())
    }

    async fn put(&self, key: &ResumeKey, checkpoint: Checkpoint) -> Result<(), StoreError> {
        let mut map = self.inner.write().await;
        map.insert(key.clone(), checkpoint);
        Ok(())
    }

    async fn delete(&self, key: &ResumeKey) -> Result<(), StoreError> {
        let mut map = self.inner.write().await;
        map.remove(key);
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use serde_json::json;
    use static_assertions::assert_impl_all;
    use tokio::task::JoinSet;
    use url::Url;

    use super::MemoryStore;
    use crate::state::{Checkpoint, ResumeKey, StateStore};

    assert_impl_all!(MemoryStore: Send, Sync, Clone);

    fn key(n: u8) -> ResumeKey {
        ResumeKey::new(
            &Url::parse("https://a/").unwrap(),
            "mars",
            &json!({"n": n}),
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn get_on_empty_returns_none() {
        let s = MemoryStore::new();
        assert!(s.get(&key(0)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn put_then_get_returns_checkpoint() {
        let s = MemoryStore::new();
        let cp = Checkpoint::new(42, Some("e@42".into()));
        s.put(&key(0), cp.clone()).await.unwrap();
        assert_eq!(s.get(&key(0)).await.unwrap(), Some(cp));
    }

    #[tokio::test]
    async fn put_overwrites() {
        let s = MemoryStore::new();
        s.put(&key(0), Checkpoint::new(1, None)).await.unwrap();
        s.put(&key(0), Checkpoint::new(2, None)).await.unwrap();
        assert_eq!(
            s.get(&key(0))
                .await
                .unwrap()
                .unwrap()
                .last_committed_sequence,
            2
        );
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let s = MemoryStore::new();
        s.put(&key(0), Checkpoint::new(1, None)).await.unwrap();
        s.delete(&key(0)).await.unwrap();
        assert!(s.get(&key(0)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_absent_key_is_ok() {
        let s = MemoryStore::new();
        s.delete(&key(0)).await.unwrap();
    }

    #[tokio::test]
    async fn cloned_handles_share_state() {
        let a = MemoryStore::new();
        let b = a.clone();
        a.put(&key(0), Checkpoint::new(7, None)).await.unwrap();
        assert_eq!(
            b.get(&key(0))
                .await
                .unwrap()
                .unwrap()
                .last_committed_sequence,
            7
        );
    }

    #[tokio::test]
    async fn concurrent_puts_to_distinct_keys_all_land() {
        let store = MemoryStore::new();
        let mut set = JoinSet::new();
        for i in 0..10u8 {
            let s = store.clone();
            let k = key(i);
            set.spawn(async move {
                s.put(&k, Checkpoint::new(u64::from(i), None))
                    .await
                    .unwrap();
            });
        }
        while let Some(joined) = set.join_next().await {
            joined.unwrap();
        }
        for i in 0..10u8 {
            let got = store.get(&key(i)).await.unwrap().unwrap();
            assert_eq!(got.last_committed_sequence, u64::from(i));
        }
    }
}
