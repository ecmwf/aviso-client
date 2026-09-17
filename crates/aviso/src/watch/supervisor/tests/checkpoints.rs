// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::*;
use crate::state::{Checkpoint, JsonFileStore, MemoryStore};
use crate::watch::Trigger;
use tokio::time::timeout;

mod gated_store;

const PROTOCOL_PROGRESS_TIMEOUT: Duration = Duration::from_secs(5);
// A whole disk case includes setup, multiple durable writes, exit flush and
// reopen. Durable filesystem I/O has no five-second latency contract.
// This is a hang guard for the integration, not a storage latency assertion.
const FILE_CASE_TIMEOUT: Duration = Duration::from_secs(60);

struct ObservedStore {
    inner: Arc<dyn StateStore>,
    activity: Arc<StoreActivity>,
}

struct StoreActivity(std::sync::Mutex<(String, std::time::Instant)>);

impl StoreActivity {
    fn record(&self, activity: String) {
        *self.0.lock().unwrap() = (activity, std::time::Instant::now());
    }

    fn describe(&self) -> String {
        let (activity, started) = &*self.0.lock().unwrap();
        format!("{activity}, {:.3}s ago", started.elapsed().as_secs_f64())
    }
}

#[async_trait::async_trait]
impl StateStore for ObservedStore {
    async fn get(&self, key: &ResumeKey) -> Result<Option<Checkpoint>, crate::state::StoreError> {
        self.activity.record("get started".into());
        let result = self.inner.get(key).await;
        self.activity.record("get finished".into());
        result
    }

    async fn put(
        &self,
        key: &ResumeKey,
        checkpoint: Checkpoint,
    ) -> Result<(), crate::state::StoreError> {
        let sequence = checkpoint.last_committed_sequence;
        self.activity.record(format!("put {sequence} started"));
        let result = self.inner.put(key, checkpoint).await;
        self.activity.record(format!("put {sequence} finished"));
        result
    }

    async fn delete(&self, key: &ResumeKey) -> Result<(), crate::state::StoreError> {
        self.inner.delete(key).await
    }
}

#[tokio::test]
async fn backward_deliveries_keep_checkpoints_monotonic() {
    for backend in ["none", "memory"] {
        check_matrix(backend).await;
    }
}

#[tokio::test]
async fn backward_deliveries_keep_file_checkpoints_monotonic_across_reopen() {
    check_matrix("json").await;
}

async fn check_matrix(backend: &str) {
    for baseline in ["fresh", "explicit", "stored"] {
        if backend == "none" && baseline == "stored" {
            continue;
        }
        for flush in [false, true] {
            for sequences in [&[6, 4, 5][..], &[6, 4, 5, 7, 5], &[6, 4, 5, 7], &[4, 5]] {
                if baseline == "fresh" && sequences == [4, 5] {
                    continue;
                }
                let activity = Arc::new(StoreActivity(std::sync::Mutex::new((
                    "setup started".into(),
                    std::time::Instant::now(),
                ))));
                let session = check_session(backend, baseline, flush, sequences, &activity);
                if backend == "json" {
                    assert!(
                        timeout(FILE_CASE_TIMEOUT, session).await.is_ok(),
                        "disk case timed out: {backend}/{baseline}/flush={flush}/{sequences:?}; storage: {}",
                        activity.describe()
                    );
                } else {
                    session.await;
                }
            }
        }
    }
}

// Disk progress is bounded by the single case watchdog, including setup and
// reopen. Protocol cases retain their short, per-operation liveness guard.
async fn progress<T>(
    backend: &str,
    future: impl std::future::Future<Output = T>,
) -> Result<T, tokio::time::error::Elapsed> {
    if backend == "json" {
        Ok(future.await)
    } else {
        timeout(PROTOCOL_PROGRESS_TIMEOUT, future).await
    }
}

async fn check_session(
    backend: &str,
    baseline: &str,
    flush: bool,
    sequences: &[u64],
    activity: &Arc<StoreActivity>,
) {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("state.json");
    let store: Option<Arc<dyn StateStore>> = match backend {
        "memory" => Some(Arc::new(MemoryStore::new())),
        "json" => Some(Arc::new(JsonFileStore::open(&file).await.unwrap())),
        _ => None,
    };
    let observed = store.map(|inner| {
        Arc::new(ObservedStore {
            inner,
            activity: Arc::clone(activity),
        })
    });
    let store = observed
        .as_ref()
        .map(|store| Arc::clone(store) as Arc<dyn StateStore>);
    let base_url = url::Url::parse(&format!("{}/", server.uri())).unwrap();
    let key = ResumeKey::new(&base_url, "mars", &json!({}), None).unwrap();
    if baseline == "stored" {
        store
            .as_ref()
            .unwrap()
            .put(&key, Checkpoint::new(6, Some("mars@6".into())))
            .await
            .unwrap();
    }
    mount_stream(&server, session_body(sequences)).await;
    let request = if baseline == "explicit" {
        WatchRequest::watch_from("mars", ResumeStart::AfterSequence(6))
    } else {
        WatchRequest::watch("mars")
    };
    let (mut rx, _cancel, handle, _drop) =
        start_supervisor_full(&server, request, store.clone(), flush);
    for sequence in sequences {
        let received = progress(backend, rx.recv()).await;
        assert!(
            received.is_ok(),
            "receive {sequence} timed out: {backend}/{baseline}/flush={flush}/{sequences:?}; storage: {}; requests: {:?}; supervisor finished: {}",
            observed
                .as_ref()
                .map_or_else(|| "none".into(), |store| store.activity.describe()),
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .map(|request| request.body_json::<serde_json::Value>().unwrap())
                .collect::<Vec<_>>(),
            handle.is_finished()
        );
        let item = received.unwrap().unwrap().unwrap();
        assert_eq!(
            item.sequence, *sequence,
            "backward items must still be delivered"
        );
    }
    assert!(matches!(
        progress(backend, rx.recv()).await.unwrap(),
        Some(Err(ClientError::Http { status: 404, .. }))
    ));
    progress(backend, handle).await.unwrap().unwrap();
    let committed = sequences[..sequences.len() - 1]
        .iter()
        .copied()
        .max()
        .unwrap()
        .max(6);
    let expected = if flush {
        sequences.iter().copied().max().unwrap().max(6)
    } else {
        committed
    };
    check_requests(&server, baseline, committed).await;
    if let Some(store) = store {
        let expected = (!(baseline == "explicit" && expected == 6)).then_some(expected);
        check_stored_checkpoint(
            store.as_ref(),
            &key,
            expected,
            (backend == "json").then_some(file.as_path()),
        )
        .await;
    }
}

async fn check_requests(server: &MockServer, baseline: &str, committed: u64) {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let initial: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    if baseline == "fresh" {
        assert!(initial.get("from_id").is_none());
    } else {
        assert_eq!(initial["from_id"], "7");
    }
    let reconnect: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(reconnect["from_id"], (committed + 1).to_string());
}

fn session_body(sequences: &[u64]) -> String {
    let mut body = String::new();
    for (index, sequence) in sequences.iter().enumerate() {
        body.push_str(&sse_chunk(
            if index == 0 {
                "replay"
            } else {
                "live-notification"
            },
            cloud_event("mars", *sequence),
        ));
    }
    body.push_str(&sse_chunk(
        "connection-closing",
        closing("max_duration_reached"),
    ));
    body
}

async fn check_stored_checkpoint(
    store: &dyn StateStore,
    key: &ResumeKey,
    expected: Option<u64>,
    file: Option<&std::path::Path>,
) {
    let checkpoint = store.get(key).await.unwrap();
    if let Some(expected) = expected {
        let checkpoint = checkpoint.as_ref().unwrap();
        assert_eq!(checkpoint.last_committed_sequence, expected);
        assert_eq!(checkpoint.last_event_id, Some(format!("mars@{expected}")));
    } else {
        assert!(
            checkpoint.is_none(),
            "do not persist items below the explicit baseline"
        );
    }
    if let Some(file) = file {
        let reopened = JsonFileStore::open(file).await.unwrap();
        assert_eq!(reopened.get(key).await.unwrap(), checkpoint);
    }
}

struct RejectWrites;

async fn mount_stream(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(move |request: &wiremock::Request| {
            let value: serde_json::Value = request.body_json().unwrap();
            opened_stream(&body, value.get("from_id").is_some())
        })
        .up_to_n_times(1)
        .expect(1)
        .mount(server)
        .await;
}

#[async_trait::async_trait]
impl StateStore for RejectWrites {
    async fn get(&self, _key: &ResumeKey) -> Result<Option<Checkpoint>, crate::state::StoreError> {
        Ok(None)
    }

    async fn put(
        &self,
        _key: &ResumeKey,
        checkpoint: Checkpoint,
    ) -> Result<(), crate::state::StoreError> {
        assert_eq!(checkpoint.last_committed_sequence, 6);
        Err(std::io::Error::other("checkpoint write rejected").into())
    }

    async fn delete(&self, _key: &ResumeKey) -> Result<(), crate::state::StoreError> {
        Ok(())
    }
}

#[tokio::test]
async fn rejected_put_stops_before_current_triggers_and_exit_flush_does_not_advance() {
    for flush in [false, true] {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}",
            sse_chunk("replay", cloud_event("mars", 6)),
            sse_chunk("live-notification", cloud_event("mars", 4))
        );
        mount_stream(&server, body).await;
        let (trigger, counter) = Trigger::test_fail_on_call(2, 0, true);
        let request = WatchRequest::watch("mars").with_triggers(vec![trigger]);
        let (mut rx, _cancel, handle, _drop) =
            start_supervisor_full(&server, request, Some(Arc::new(RejectWrites)), flush);
        assert_eq!(
            timeout(Duration::from_secs(5), rx.recv())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .sequence,
            6
        );
        assert!(matches!(
            timeout(Duration::from_secs(5), rx.recv()).await.unwrap(),
            Some(Err(ClientError::StateStore(_)))
        ));
        timeout(Duration::from_secs(5), handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(rx.recv().await.is_none());
    }
}

#[tokio::test]
async fn failed_forward_trigger_never_becomes_pending_even_with_exit_flush() {
    for flush in [false, true] {
        let server = MockServer::start().await;
        let mut body = String::new();
        for sequence in [6, 4, 5, 7] {
            body.push_str(&sse_chunk(
                "live-notification",
                cloud_event("mars", sequence),
            ));
        }
        mount_stream(&server, body).await;
        let store: Arc<dyn StateStore> = Arc::new(MemoryStore::new());
        let (trigger, counter) = Trigger::test_fail_on_call(4, 0, true);
        let request = WatchRequest::watch("mars").with_triggers(vec![trigger]);
        let (mut rx, _cancel, handle, _drop) =
            start_supervisor_full(&server, request, Some(store.clone()), flush);
        for sequence in [6, 4, 5] {
            assert_eq!(
                timeout(Duration::from_secs(5), rx.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .sequence,
                sequence
            );
        }
        assert!(matches!(
            timeout(Duration::from_secs(5), rx.recv()).await.unwrap(),
            Some(Err(ClientError::TriggerFailed { .. }))
        ));
        timeout(Duration::from_secs(5), handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 4);
        let base_url = url::Url::parse(&format!("{}/", server.uri())).unwrap();
        let key = ResumeKey::new(&base_url, "mars", &json!({}), None).unwrap();
        let cp = store.get(&key).await.unwrap().unwrap();
        assert_eq!(cp.last_committed_sequence, 6);
        assert_eq!(cp.last_event_id.as_deref(), Some("mars@6"));
    }
}
