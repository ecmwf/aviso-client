//! Watch supervisor: opens one HTTP POST against the watch or replay
//! endpoint, parses the SSE response, decodes `CloudEvent`s, drives the
//! [`WatchState`] reducer, and forwards [`Notification`]s on a bounded
//! channel until the connection closes cleanly or a fatal condition
//! terminates it.
//!
//! Resilience (reconnect loop, auth refresh, heartbeat watchdog, HTTP
//! non-200 status mapping into reducer events, `StateStore` checkpoint
//! advancement) is intentionally out of scope here; a follow-up commit
//! adds it on top of this happy-path data path. The single-connection
//! exit conditions in this file are:
//!
//! - The server emits a `connection-closing` event (any reason); the
//!   reducer transitions to `Reconnecting` and this supervisor exits.
//! - The server emits an `error` event or an unknown
//!   `connection-closing.reason`; the consumer sees
//!   [`ClientError::StreamProtocol`].
//! - A `CloudEvent` id is malformed; the consumer sees
//!   [`ClientError::MalformedEvent`].
//! - A sequence gap is detected; the consumer sees
//!   [`ClientError::HistoryGap`] and the stream terminates.
//! - The consumer drops the stream; the supervisor observes cancellation
//!   and exits cleanly.

use std::sync::Arc;

use reqwest::header::AUTHORIZATION;
use tokio::sync::{mpsc, oneshot};
use url::Url;

use super::wire::{
    WireCloudEvent, WireConnectionClosing, WireConnectionEstablished, WireErrorEvent,
    WireReplayControl, WireWatchRequest,
};
use super::{
    ConnectionLossReason, FatalKind, GapReason, ResumeStart, ServerCloseReason, WatchEvent,
    WatchMode, WatchRequest, WatchState,
};
use crate::auth::AuthProvider;
use crate::{ClientError, Notification, parse_cloudevent_id};

/// Internal channel capacity for the supervisor's notification mpsc. See
/// the [`super::NotificationStream`] doc comment for the backpressure
/// contract this constant participates in.
pub(crate) const CHANNEL_CAPACITY: usize = 128;

/// Drive a single watch session to completion. Owns its inputs by value;
/// the spawn caller in [`crate::client::AvisoClient::watch`] passes cloned
/// or `Arc`-shared handles so the task is `'static` without forcing the
/// supervisor to keep an [`crate::AvisoClient`] alive (which would defeat
/// parent-cancellation work in the follow-up resilience commit).
pub(crate) async fn run_supervisor(
    request: WatchRequest,
    http: reqwest::Client,
    base_url: Url,
    auth: Option<Arc<dyn AuthProvider>>,
    tx: mpsc::Sender<Result<Notification, ClientError>>,
    mut cancel: oneshot::Receiver<()>,
) {
    let mut state = match initial_state(&request) {
        Ok(s) => s,
        Err(e) => {
            let _ = send_or_cancel(&tx, Err(e), &mut cancel).await;
            return;
        }
    };

    match run_one_connection(
        &mut state,
        &request,
        &http,
        &base_url,
        auth.as_ref(),
        &tx,
        &mut cancel,
    )
    .await
    {
        Ok(()) => {}
        Err(terminal_err) => {
            let _ = send_or_cancel(&tx, Err(terminal_err), &mut cancel).await;
        }
    }
}

fn initial_state(request: &WatchRequest) -> Result<WatchState, ClientError> {
    match (request.mode(), request.from().cloned()) {
        (WatchMode::Watch, from) => Ok(WatchState::watch(from)),
        (WatchMode::ReplayOnly, Some(from)) => Ok(WatchState::replay_only(from)),
        (WatchMode::ReplayOnly, None) => Err(ClientError::Config(
            "replay-only watch requires a resume position".into(),
        )),
    }
}

/// Drain the single HTTP connection that backs this watch.
///
/// Returns `Ok(())` for clean closes (consumer drop, server-driven close
/// like `end_of_stream`, terminal reducer state reached without an error
/// to surface) and `Err(_)` for everything that should reach the consumer
/// as a typed `ClientError` item on the stream.
#[allow(
    clippy::too_many_arguments,
    reason = "the supervisor's collaborators are intentionally passed by reference rather than bundled into a context struct, so each await point owns clear borrows"
)]
async fn run_one_connection(
    state: &mut WatchState,
    request: &WatchRequest,
    http: &reqwest::Client,
    base_url: &Url,
    auth: Option<&Arc<dyn AuthProvider>>,
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    cancel: &mut oneshot::Receiver<()>,
) -> Result<(), ClientError> {
    let endpoint = match request.mode() {
        WatchMode::Watch => "api/v1/watch",
        WatchMode::ReplayOnly => "api/v1/replay",
    };
    let url = base_url.join(endpoint).map_err(|e| {
        ClientError::Config(format!(
            "build watch endpoint url from base {base_url} and path {endpoint:?}: {e}"
        ))
    })?;
    let body = WireWatchRequest::from_public(request)?;

    let auth_header = match auth {
        Some(provider) => Some(tokio::select! {
            biased;
            _ = &mut *cancel => return Ok(()),
            v = provider.authorization_header() => v?,
        }),
        None => None,
    };

    let mut builder = http.post(url).json(&body);
    if let Some(value) = auth_header {
        builder = builder.header(AUTHORIZATION, value);
    }

    let mut response = tokio::select! {
        biased;
        _ = &mut *cancel => return Ok(()),
        r = builder.send() => r?,
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .map(String::from);
        let body_bytes = tokio::select! {
            biased;
            _ = &mut *cancel => return Ok(()),
            b = response.bytes() => b?,
        };
        let body = String::from_utf8_lossy(&body_bytes).into_owned();
        return Err(ClientError::Http {
            status,
            body,
            request_id,
        });
    }

    let _ = state.transition(WatchEvent::ConnectionEstablished);

    let mut parser = finesse::Parser::new();
    let mut gap_guard = GapGuard::starting_from(request.from());

    loop {
        let chunk = tokio::select! {
            biased;
            _ = &mut *cancel => {
                let _ = state.transition(WatchEvent::Stop);
                return Ok(());
            }
            c = response.chunk() => c,
        };
        let eof = matches!(chunk, Ok(None));
        match chunk {
            Ok(Some(bytes)) => parser.feed(&bytes),
            Ok(None) => parser.end(),
            Err(transport_e) => return Err(ClientError::Transport(transport_e)),
        }
        match drain_frames(&mut parser, state, &mut gap_guard, tx, cancel).await? {
            DrainOutcome::Continue => {}
            DrainOutcome::ServerClosed | DrainOutcome::StopRequested => return Ok(()),
        }
        if state.is_terminal() {
            return Ok(());
        }
        if eof {
            // The wire ended without a `connection-closing` frame. That is
            // a transport-level abnormality; surface it as a typed error so
            // a consumer can tell "the server told me it was done" (clean
            // close, returns None) apart from "the connection went dark"
            // (this error). The reducer is told too so the state machine
            // captures the loss reason for any downstream observability.
            let _ = state.transition(WatchEvent::ConnectionLost {
                reason: ConnectionLossReason::UnexpectedEof,
            });
            return Err(ClientError::StreamProtocol {
                message: "stream ended without a connection-closing frame".to_string(),
                request_id: None,
            });
        }
    }
}

/// Result of one call to [`drain_frames`].
///
/// `ServerClosed` is the supervisor's signal that a known
/// `connection-closing` frame was observed; in this single-connection
/// version of the supervisor, that always means "exit cleanly". A future
/// reconnect-loop revision will replace this with a richer signal that
/// carries the close reason.
///
/// `StopRequested` carries the "consumer is gone or cancellation fired"
/// signal up to [`run_one_connection`] so it does not re-poll the cancel
/// `oneshot::Receiver` after it has already resolved (which would panic).
#[derive(Debug, PartialEq, Eq)]
enum DrainOutcome {
    Continue,
    ServerClosed,
    StopRequested,
}

/// Drain every frame currently queued in the SSE parser and feed it
/// through the supervisor's mapping table.
///
/// Returns `Err(_)` only for conditions the consumer must see as a
/// terminal error: a malformed `CloudEvent` id, a server `error` event,
/// an unknown `connection-closing.reason`, or a JSON decode failure.
/// Routine frame handling returns `Ok(())` and the caller decides
/// whether to keep draining the wire.
#[allow(
    clippy::too_many_lines,
    reason = "the SSE-event-type dispatch is a flat match over six wire variants; splitting each branch into its own helper trades readability for line count, and reviewers want to see the full mapping table in one place"
)]
async fn drain_frames(
    parser: &mut finesse::Parser,
    state: &mut WatchState,
    gap_guard: &mut GapGuard,
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    cancel: &mut oneshot::Receiver<()>,
) -> Result<DrainOutcome, ClientError> {
    while let Some(frame) = parser.next_frame() {
        let finesse::Frame::Message(msg) = frame else {
            continue;
        };
        match msg.event.as_str() {
            "live-notification" | "replay" => {
                let raw: serde_json::Value = serde_json::from_str(&msg.data)?;
                let top_type = raw.get("type").and_then(|v| v.as_str());
                if top_type == Some("connection_established") {
                    let _: WireConnectionEstablished = serde_json::from_value(raw)?;
                    let _ = state.transition(WatchEvent::ConnectionEstablished);
                    continue;
                }
                let wire: WireCloudEvent = serde_json::from_value(raw)?;
                let (event_type, sequence) = match parse_cloudevent_id(&wire.id) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = state.transition(WatchEvent::Fatal(FatalKind::MalformedEvent));
                        return Err(e);
                    }
                };
                let _ = state.transition(WatchEvent::NotificationReceived { sequence });
                match gap_guard.observe(sequence) {
                    Ok(()) => {
                        let notification = Notification {
                            event_type,
                            sequence,
                            identifier: wire.data.identifier,
                            payload: wire.data.payload,
                            request_id: None,
                        };
                        if send_or_cancel(tx, Ok(notification), cancel).await.is_err() {
                            return Ok(DrainOutcome::StopRequested);
                        }
                    }
                    Err(reason) => {
                        let _ = state.transition(WatchEvent::GapDetected(reason));
                        let _ = send_or_cancel(tx, Err(ClientError::HistoryGap { reason }), cancel)
                            .await;
                        return Ok(DrainOutcome::StopRequested);
                    }
                }
            }
            "heartbeat" => {
                let _ = state.transition(WatchEvent::HeartbeatReceived);
            }
            "replay-control" => {
                let wire: WireReplayControl = serde_json::from_str(&msg.data)?;
                match wire.tag.as_str() {
                    "replay_completed" => {
                        let _ = state.transition(WatchEvent::ReplayCompleted);
                    }
                    "notification_replay_limit_reached" => {
                        let max_allowed = wire.max_allowed.unwrap_or(0);
                        let reason = GapReason::ReplayLimitReached { max_allowed };
                        let _ = state.transition(WatchEvent::GapDetected(reason));
                        let _ = send_or_cancel(tx, Err(ClientError::HistoryGap { reason }), cancel)
                            .await;
                        return Ok(DrainOutcome::StopRequested);
                    }
                    _other => {}
                }
            }
            "connection-closing" => {
                let wire: WireConnectionClosing = serde_json::from_str(&msg.data)?;
                let reason = match wire.reason.as_str() {
                    "server_shutdown" => ServerCloseReason::ServerShutdown,
                    "max_duration_reached" => ServerCloseReason::MaxDurationReached,
                    "end_of_stream" => ServerCloseReason::EndOfStream,
                    other => {
                        let message = format!("unknown connection-closing reason: {other}");
                        let _ = state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                            message.clone(),
                        )));
                        return Err(ClientError::StreamProtocol {
                            message,
                            request_id: wire.request_id,
                        });
                    }
                };
                let _ = state.transition(WatchEvent::ServerClose { reason });
                // Every recognised `connection-closing` reason ends this
                // watch session in the single-connection supervisor. The
                // reducer's `Reconnect` outcome is intentionally ignored
                // here; the reconnect loop driven by `ReconnectPolicy` is
                // a separate, follow-up piece of code.
                return Ok(DrainOutcome::ServerClosed);
            }
            "error" => {
                let wire: WireErrorEvent = serde_json::from_str(&msg.data)?;
                let message = wire
                    .message
                    .clone()
                    .or_else(|| wire.error.clone())
                    .unwrap_or_else(|| "server error event".to_string());
                let _ = state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                    message.clone(),
                )));
                return Err(ClientError::StreamProtocol {
                    message,
                    request_id: wire.request_id,
                });
            }
            _other => {
                // Unknown event type: silently ignored in this iteration; a
                // future commit may add a TRACE log or a strict-mode
                // termination. Reducer state unchanged.
            }
        }
    }
    Ok(DrainOutcome::Continue)
}

/// Send `msg` on `tx`, racing with cancellation.
///
/// Returns `Ok(())` on successful send. Returns `Err(())` when either the
/// consumer dropped the receiver (`mpsc::SendError`) or the cancellation
/// oneshot fired; both cases mean "stop draining and exit cleanly". The
/// caller treats both as the same outcome.
async fn send_or_cancel(
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    msg: Result<Notification, ClientError>,
    cancel: &mut oneshot::Receiver<()>,
) -> Result<(), ()> {
    tokio::select! {
        biased;
        _ = &mut *cancel => Err(()),
        result = tx.send(msg) => result.map_err(|_| ()),
    }
}

/// Sequence-jump detector. Holds the next expected sequence; updates on
/// every observation; reports a gap when the observed sequence is strictly
/// greater than expected.
///
/// Sequence going backwards (observed less than expected) is treated as a
/// duplicate or server-side anomaly and ignored at this layer; a future
/// commit may surface it as a structured WARN log.
struct GapGuard {
    expected: Option<u64>,
}

impl GapGuard {
    fn starting_from(from: Option<&ResumeStart>) -> Self {
        let expected = match from {
            Some(ResumeStart::AfterSequence(n)) => n.checked_add(1),
            _ => None,
        };
        Self { expected }
    }

    fn observe(&mut self, observed: u64) -> Result<(), GapReason> {
        match self.expected {
            None => {
                self.expected = observed.checked_add(1);
                Ok(())
            }
            Some(expected) if observed == expected => {
                self.expected = observed.checked_add(1);
                Ok(())
            }
            Some(expected) if observed > expected => {
                Err(GapReason::SequenceJump { expected, observed })
            }
            Some(_) => Ok(()),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    reason = "test code: panic-on-unexpected is the standard test diagnostic; small helpers move JSON values into wiremock bodies"
)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use tokio::sync::{mpsc, oneshot};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{
        AuthProvider, ClientError, GapGuard, GapReason, Notification, ResumeStart, WatchRequest,
        run_supervisor, send_or_cancel,
    };
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

    fn start_supervisor(
        server: &MockServer,
        request: WatchRequest,
    ) -> (
        mpsc::Receiver<Result<Notification, ClientError>>,
        oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let (tx, rx) = mpsc::channel(super::CHANNEL_CAPACITY);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let base_url = url::Url::parse(&format!("{}/", server.uri())).unwrap();
        let http = reqwest::Client::builder().build().unwrap();
        let no_auth: Option<Arc<dyn AuthProvider>> = None;
        let handle = tokio::spawn(run_supervisor(
            request, http, base_url, no_auth, tx, cancel_rx,
        ));
        (rx, cancel_tx, handle)
    }

    #[tokio::test]
    async fn drop_cancel_oneshot_makes_supervisor_exit_within_bounded_time() {
        // The supervisor must observe cancellation even while it is parked
        // on a bounded-channel `tx.send().await` because the consumer fell
        // behind. The test constructs that exact situation: a response body
        // carrying many more notifications than the channel capacity, no
        // terminating frame, a consumer that takes one item and then stops.
        // Once the channel fills, the supervisor blocks on `send_or_cancel`.
        // Dropping the cancel sender then fires the `select!` arm and the
        // supervisor exits cleanly within 500 ms.
        let server = MockServer::start().await;
        let notification_count = super::CHANNEL_CAPACITY + 64;
        let mut body = String::new();
        for n in 1..=notification_count {
            body.push_str(&sse_chunk(
                "live-notification",
                cloud_event("mars", n as u64),
            ));
        }
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let (mut rx, cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));

        let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("first notification should arrive promptly")
            .expect("channel must not close before the first item");
        assert!(
            first.is_ok(),
            "first item should be Ok(Notification): {first:?}"
        );

        drop(cancel_tx);

        tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("supervisor must exit within 500ms of cancellation")
            .expect("supervisor task panicked");

        drop(rx);
    }

    #[tokio::test]
    async fn drain_frames_mapping_covers_live_notification_with_cloudevent() {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}",
            sse_chunk("live-notification", cloud_event("mars", 7)),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;

        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.event_type, "mars");
        assert_eq!(first.sequence, 7);
        assert!(
            rx.recv().await.is_none(),
            "stream should close after end_of_stream"
        );
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_treats_connection_established_marker_as_control() {
        let server = MockServer::start().await;
        let marker = json!({
            "type": "connection_established",
            "topic": "mars",
            "timestamp": "2026-05-17T12:00:00Z",
            "connection_will_close_in_seconds": 3600u64,
            "request_id": "req-est"
        });
        let body = format!(
            "{}{}{}",
            sse_chunk("live-notification", marker),
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(
            first.sequence, 1,
            "the connection_established marker must not emit a notification"
        );
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_does_not_confuse_payload_substring_with_marker() {
        // Regression: the disambiguation must read the *top-level* `type`
        // field; a CloudEvent whose payload contains the literal string
        // `"connection_established"` must still decode as a Notification.
        let server = MockServer::start().await;
        let ev = cloud_event_with_payload(
            "mars",
            5,
            json!({ "free_text": "connection_established was emitted earlier today" }),
        );
        let body = format!(
            "{}{}",
            sse_chunk("live-notification", ev),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.event_type, "mars");
        assert_eq!(first.sequence, 5);
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_terminates_on_error_event_with_stream_protocol_error() {
        let server = MockServer::start().await;
        let err = json!({
            "error": "stream_processing_failed",
            "message": "boom",
            "topic": "mars",
            "request_id": "req-err"
        });
        let body = sse_chunk("error", err);
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let item = rx.recv().await.unwrap();
        match item {
            Err(ClientError::StreamProtocol {
                message,
                request_id,
            }) => {
                assert_eq!(message, "boom");
                assert_eq!(request_id.as_deref(), Some("req-err"));
            }
            other => panic!("expected StreamProtocol, got {other:?}"),
        }
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_terminates_on_unknown_connection_closing_reason() {
        let server = MockServer::start().await;
        let body = sse_chunk(
            "connection-closing",
            closing("future_reason_we_did_not_anticipate"),
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let item = rx.recv().await.unwrap();
        assert!(
            matches!(item, Err(ClientError::StreamProtocol { .. })),
            "got {item:?}"
        );
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_handles_heartbeat_as_observation_only() {
        let server = MockServer::start().await;
        let heartbeat = json!({ "timestamp": "2026-05-17T12:00:00Z", "topic": "mars" });
        let body = format!(
            "{}{}{}",
            sse_chunk("heartbeat", heartbeat),
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.sequence, 1);
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_terminates_on_replay_limit_reached_with_history_gap() {
        let server = MockServer::start().await;
        let limit = json!({
            "type": "notification_replay_limit_reached",
            "topic": "mars",
            "max_allowed": 1000u64,
            "timestamp": "2026-05-17T12:00:00Z"
        });
        let body = sse_chunk("replay-control", limit);
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let item = rx.recv().await.unwrap();
        match item {
            Err(ClientError::HistoryGap {
                reason: GapReason::ReplayLimitReached { max_allowed },
            }) => {
                assert_eq!(max_allowed, 1000);
            }
            other => panic!("expected HistoryGap{{ReplayLimitReached}}, got {other:?}"),
        }
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn connection_closing_terminates_before_any_following_frames() {
        // Even if the server keeps writing notifications after a
        // `connection-closing` frame on the same wire (which it should not,
        // but which is exactly the bug class the supervisor must guard
        // against), the supervisor must stop at the close frame and not
        // surface the trailing notifications.
        let server = MockServer::start().await;
        let body = format!(
            "{}{}{}",
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("connection-closing", closing("end_of_stream")),
            sse_chunk("live-notification", cloud_event("mars", 99)),
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.sequence, 1);
        assert!(
            rx.recv().await.is_none(),
            "the post-close-frame notification must NOT surface"
        );
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn unexpected_eof_without_close_frame_terminates_with_stream_protocol_error() {
        let server = MockServer::start().await;
        // One notification, then the response body ends (no close frame).
        let body = sse_chunk("live-notification", cloud_event("mars", 1));
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.sequence, 1);
        let item = rx.recv().await.unwrap();
        assert!(
            matches!(item, Err(ClientError::StreamProtocol { .. })),
            "got {item:?}"
        );
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn drain_frames_mapping_silently_ignores_unknown_sse_event_types() {
        let server = MockServer::start().await;
        let body = format!(
            "{}{}{}",
            sse_chunk("future-event-we-do-not-know", json!({ "anything": true })),
            sse_chunk("live-notification", cloud_event("mars", 1)),
            sse_chunk("connection-closing", closing("end_of_stream"))
        );
        Mock::given(method("POST"))
            .and(path("/api/v1/watch"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        let (mut rx, _cancel_tx, handle) = start_supervisor(&server, WatchRequest::watch("mars"));
        let first = rx.recv().await.unwrap().unwrap();
        assert_eq!(first.sequence, 1);
        assert!(rx.recv().await.is_none());
        handle.await.unwrap();
    }

    #[test]
    fn gap_guard_no_resume_adopts_first_observed_sequence() {
        let mut g = GapGuard::starting_from(None);
        assert!(g.observe(100).is_ok());
        assert!(g.observe(101).is_ok());
        assert!(matches!(
            g.observe(105),
            Err(GapReason::SequenceJump {
                expected: 102,
                observed: 105
            })
        ));
    }

    #[test]
    fn gap_guard_after_sequence_expects_next() {
        let mut g = GapGuard::starting_from(Some(&ResumeStart::AfterSequence(9)));
        assert!(g.observe(10).is_ok());
        assert!(g.observe(11).is_ok());
        assert!(matches!(
            g.observe(13),
            Err(GapReason::SequenceJump {
                expected: 12,
                observed: 13
            })
        ));
    }

    #[test]
    fn gap_guard_tolerates_backwards_sequence() {
        let mut g = GapGuard::starting_from(None);
        assert!(g.observe(100).is_ok());
        assert!(
            g.observe(50).is_ok(),
            "backwards observation must not be reported as a gap"
        );
    }

    #[test]
    fn gap_guard_overflow_resets_to_fresh_start() {
        let mut g = GapGuard::starting_from(Some(&ResumeStart::AfterSequence(u64::MAX)));
        assert!(
            g.observe(0).is_ok(),
            "overflow on construction must not poison subsequent observation"
        );
    }

    #[tokio::test]
    async fn send_or_cancel_returns_err_when_cancel_fires_first() {
        let (tx, _rx) = mpsc::channel(1);
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        drop(cancel_tx);
        let result = send_or_cancel(
            &tx,
            Err(ClientError::Config("dummy".into())),
            &mut cancel_rx,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_or_cancel_returns_err_when_receiver_is_dropped() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let (_cancel_tx, mut cancel_rx) = oneshot::channel();
        let result = send_or_cancel(
            &tx,
            Err(ClientError::Config("dummy".into())),
            &mut cancel_rx,
        )
        .await;
        assert!(result.is_err());
    }
}
