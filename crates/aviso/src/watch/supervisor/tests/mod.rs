use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{
    CHANNEL_CAPACITY, GapGuard, heartbeat_starvation_budget, run_supervisor, send_or_cancel,
};
use crate::auth::AuthProvider;
use crate::state::{ResumeKey, StateStore};
use crate::watch::{GapReason, ResumeStart, WatchRequest};
use crate::{ClientError, Notification};
use std::sync::Arc;

fn sse_chunk(event_type: &str, data: serde_json::Value) -> String {
    format!("event: {event_type}\ndata: {data}\n\n")
}

fn cloud_event(event_type: &str, sequence: u64) -> serde_json::Value {
    json!({
        "id": format!("{event_type}@{sequence}"),
        "source": "https://aviso.example",
        "type": format!("int.ecmwf.aviso.{event_type}"),
        "time": "2026-05-17T12:34:56Z",
        "data": {
            "identifier": { "country": "UK" },
            "payload": { "n": sequence }
        }
    })
}

fn cloud_event_with_payload(
    event_type: &str,
    sequence: u64,
    payload: serde_json::Value,
) -> serde_json::Value {
    json!({
        "id": format!("{event_type}@{sequence}"),
        "type": format!("int.ecmwf.aviso.{event_type}"),
        "data": {
            "identifier": {},
            "payload": payload
        }
    })
}

fn closing(reason: &str) -> serde_json::Value {
    json!({
        "reason": reason,
        "timestamp": "2026-05-17T13:00:00Z",
        "message": "",
        "topic": "mars",
        "request_id": "req-close"
    })
}

/// Test helper that spawns a `run_supervisor` task against a `MockServer`.
///
/// Returns the consumer-side mpsc receiver, the per-stream cancel
/// sender, the supervisor `JoinHandle`, and the parent-drop `watch`
/// sender. The fourth value is the load-bearing test fixture for the
/// parent-cancel cascade: as long as the test holds it, the
/// supervisor's `parent_cancel.changed()` arm stays parked. Tests
/// that want to trigger the cascade drop it explicitly; tests that
/// do not, simply bind it to a name and let RAII drop it at scope
/// end (after the supervisor `JoinHandle` has already completed,
/// so the dangling sender drop never fires the cascade against an
/// already-exited supervisor).
#[allow(
    clippy::type_complexity,
    reason = "test helper's return tuple aggregates the four channels the supervisor needs (notification receiver, per-stream cancel, JoinHandle, parent-drop sender); each component is named at the call site via destructuring so the complexity does not propagate"
)]
fn start_supervisor(
    server: &MockServer,
    request: WatchRequest,
) -> (
    mpsc::Receiver<Result<Notification, ClientError>>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
    tokio::sync::watch::Sender<bool>,
) {
    start_supervisor_with_store(server, request, None)
}

/// Variant of [`start_supervisor`] that wires a state store for the
/// supervisor's commit-on-next-send cursor advancement. Returns the
/// same 4-tuple; the test recomputes the resume key from the server
/// URL and event type for `store.get(...)` queries.
#[allow(
    clippy::type_complexity,
    reason = "test helper's return tuple is unchanged from start_supervisor; only the state-store wiring differs"
)]
fn start_supervisor_with_store(
    server: &MockServer,
    request: WatchRequest,
    store: Option<Arc<dyn StateStore>>,
) -> (
    mpsc::Receiver<Result<Notification, ClientError>>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
    tokio::sync::watch::Sender<bool>,
) {
    let capacity = if store.is_some() {
        1
    } else {
        super::CHANNEL_CAPACITY
    };
    let (tx, rx) = mpsc::channel(capacity);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    let base_url = url::Url::parse(&format!("{}/", server.uri())).unwrap();
    let http = reqwest::Client::builder().build().unwrap();
    let no_auth: Option<Arc<dyn AuthProvider>> = None;
    let heartbeat_interval = std::time::Duration::from_secs(30);
    let resume_key = ResumeKey::new(&base_url, request.event_type(), &json!({}), None).unwrap();
    let (drop_sender, parent_cancel) = tokio::sync::watch::channel(false);
    let active_resume_keys = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<
        ResumeKey,
        usize,
    >::new()));
    let handle = tokio::spawn(run_supervisor(
        request,
        http,
        base_url,
        no_auth,
        heartbeat_interval,
        store,
        resume_key,
        tx,
        cancel_rx,
        parent_cancel,
        active_resume_keys,
    ));
    (rx, cancel_tx, handle, drop_sender)
}

#[cfg(test)]
mod cancellation;
#[cfg(test)]
mod connection;
#[cfg(test)]
mod drain_mapping;
#[cfg(test)]
mod gap_guard;
#[cfg(test)]
mod heartbeat;
#[cfg(test)]
mod state_store;
