// SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::state::StoreError;
use std::sync::atomic::Ordering;
use tokio::sync::{Notify, Semaphore};

struct GatedStore {
    inner: MemoryStore,
    entered: Notify,
    release: Semaphore,
}

#[async_trait::async_trait]
impl StateStore for GatedStore {
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, StoreError> {
        self.inner.get(key).await
    }

    async fn put(&self, key: &ResumeKey, checkpoint: Checkpoint) -> Result<(), StoreError> {
        assert_eq!(checkpoint.last_committed_sequence, 6);
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
        self.inner.put(key, checkpoint).await
    }

    async fn delete(&self, key: &ResumeKey) -> Result<(), StoreError> {
        self.inner.delete(key).await
    }
}

#[tokio::test]
async fn pending_commit_blocks_current_triggers_and_delivery_until_released() {
    check_gated_commit(true).await;
}

#[tokio::test]
async fn stuck_store_hits_protocol_watchdog() {
    check_gated_commit(false).await;
}

async fn check_gated_commit(release_write: bool) {
    let server = MockServer::start().await;
    mount_stream(&server, session_body(&[6, 4])).await;
    let store = Arc::new(GatedStore {
        inner: MemoryStore::new(),
        entered: Notify::new(),
        release: Semaphore::new(0),
    });
    let (trigger, counter) = Trigger::test_fail_on_call(3, 0, true);
    let request = WatchRequest::watch("mars").with_triggers(vec![trigger]);
    let (mut rx, _cancel, handle, _drop) =
        start_supervisor_full(&server, request, Some(store.clone()), false);
    assert_eq!(
        progress("memory", rx.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .sequence,
        6
    );
    progress("memory", store.entered.notified()).await.unwrap();

    // Pause only after HTTP startup and entry into put. The closed semaphore
    // makes the ordering check deterministic without sleeping for disk I/O.
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    assert!(progress("memory", rx.recv()).await.is_err());
    // Tokio rounds timer deadlines up to its millisecond tick.
    assert!(
        (PROTOCOL_PROGRESS_TIMEOUT..=PROTOCOL_PROGRESS_TIMEOUT + Duration::from_millis(1))
            .contains(&started.elapsed())
    );
    assert!(!handle.is_finished());
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let base_url = url::Url::parse(&format!("{}/", server.uri())).unwrap();
    let key = ResumeKey::new(&base_url, "mars", &json!({}), None).unwrap();
    assert!(store.get(&key).await.unwrap().is_none());
    tokio::time::resume();

    if release_write {
        store.release.add_permits(1);
        assert_eq!(
            progress("memory", rx.recv())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .sequence,
            4
        );
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert_eq!(
            store.get(&key).await.unwrap(),
            Some(Checkpoint::new(6, Some("mars@6".into())))
        );
        assert!(matches!(
            progress("memory", rx.recv()).await.unwrap(),
            Some(Err(ClientError::Http { status: 404, .. }))
        ));
        progress("memory", handle).await.unwrap().unwrap();
        check_requests(&server, "fresh", 6).await;
    } else {
        // An in-progress put deliberately ignores stream cancellation. This
        // mock has only async work and no disk write, so aborting it is safe
        // cleanup for a gate that never opens.
        handle.abort();
        assert!(
            progress("memory", handle)
                .await
                .unwrap()
                .unwrap_err()
                .is_cancelled()
        );
        assert!(store.get(&key).await.unwrap().is_none());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
    assert!(progress("memory", rx.recv()).await.unwrap().is_none());
}
