// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use std::sync::Arc;

use reqwest::header::AUTHORIZATION;
use tokio::sync::{mpsc, oneshot, watch};
use url::Url;

use super::drain::drain_frames;
use super::opening;
use super::{
    ConnectionOutcome, DrainOutcome, GapGuard, PendingCommit, apply_outcome,
    heartbeat_starvation_budget,
};
use crate::auth::AuthProvider;
use crate::state::{ResumeKey, StateStore};
use crate::watch::retry_after;
use crate::watch::wire::WireWatchRequest;
use crate::watch::{
    ConnectionLossReason, FatalKind, ReconnectPolicy, ResumeStart, WatchEvent, WatchMode,
    WatchRequest, WatchState,
};
use crate::{ClientError, Notification};

/// Drive one HTTP connection: open the request, classify the initial
/// response, decode chunks, feed them through the SSE parser plus
/// [`drain_frames`], and return a [`ConnectionOutcome`] the outer
/// supervisor loop dispatches on.
///
/// Returns:
/// - `ServerClosed` when the wire delivered a recognised
///   `connection-closing` frame; the reducer has transitioned via
///   `WatchEvent::ServerClose` and the outer loop reads the resulting
///   state to decide whether to reconnect or terminate.
/// - `HttpStatus { status, body, request_id, retry_after }` when the
///   initial response was not exactly `200 OK`. The reducer is NOT
///   advanced here; the outer loop classifies the status.
/// - `TransportError(_)` on a reqwest-level error (TLS, connect,
///   mid-stream read).
/// - `UnexpectedEof` when the response body ended without a
///   `connection-closing` frame; the reducer has transitioned via
///   `WatchEvent::ConnectionLost { reason: UnexpectedEof }`.
/// - `HeartbeatStarved` when the per-chunk `timeout(budget, ...)`
///   fired; the reducer has transitioned via
///   `WatchEvent::HeartbeatStarvation`.
/// - `Fatal(ClientError)` for terminal wire-level conditions surfaced
///   by [`drain_frames`] (malformed `CloudEvent` id, server `error`
///   event, unknown `connection-closing.reason`, decode failure,
///   gap detected, state-store failure).
/// - `Cancelled` when per-stream drop or parent drop fired during any
///   await.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the supervisor's collaborators are intentionally passed by reference rather than bundled into a context struct, so each await point owns clear borrows; the function's length is the natural shape of a single-connection runner that maps every initial-response and chunk-loop branch to a `ConnectionOutcome`, and splitting it further obscures the mapping table; threading triggers and trigger_states as additional references keeps the same shape and the cancel-safety analysis local to each await point"
)]
pub(super) async fn run_one_connection(
    state: &mut WatchState,
    last_reconnect_policy: &mut Option<ReconnectPolicy>,
    request: &WatchRequest,
    wire_from: Option<&ResumeStart>,
    commit_cursor: &mut Option<u64>,
    pending_commit: &mut Option<PendingCommit>,
    state_store: Option<&Arc<dyn StateStore>>,
    resume_key: &ResumeKey,
    http: &reqwest::Client,
    base_url: &Url,
    auth: Option<&Arc<dyn AuthProvider>>,
    heartbeat_interval: std::time::Duration,
    retry_counter: &mut u32,
    trigger_states: &mut [crate::watch::trigger::TriggerState],
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    cancel: &mut oneshot::Receiver<()>,
    parent_cancel: &mut watch::Receiver<bool>,
    ready: &watch::Sender<bool>,
) -> ConnectionOutcome {
    let budget = heartbeat_starvation_budget(heartbeat_interval);
    let endpoint = match request.mode() {
        WatchMode::Watch => "api/v1/watch",
        WatchMode::ReplayOnly => "api/v1/replay",
    };
    let url = match base_url.join(endpoint) {
        Ok(u) => u,
        Err(e) => {
            return ConnectionOutcome::Fatal(ClientError::Config(format!(
                "build watch endpoint url for {endpoint:?}: {e}"
            )));
        }
    };
    let body = match WireWatchRequest::from_parts(request.event_type(), request.filter(), wire_from)
    {
        Ok(b) => b,
        Err(e) => return ConnectionOutcome::Fatal(e),
    };

    let auth_header = match auth {
        Some(provider) => {
            let result = tokio::select! {
                biased;
                _ = parent_cancel.changed() => return ConnectionOutcome::Cancelled,
                _ = &mut *cancel => return ConnectionOutcome::Cancelled,
                v = provider.authorization_header() => v,
            };
            match result {
                Ok(v) => Some(v),
                Err(e) => return ConnectionOutcome::Fatal(e),
            }
        }
        None => None,
    };

    let mut builder = http.post(url).json(&body);
    if let Some(value) = auth_header {
        builder = builder.header(AUTHORIZATION, value);
    }

    let opening_deadline = tokio::time::Instant::now() + opening::OPENING_TIMEOUT;
    let send_result = tokio::select! {
        biased;
        _ = parent_cancel.changed() => return ConnectionOutcome::Cancelled,
        _ = &mut *cancel => return ConnectionOutcome::Cancelled,
        () = tokio::time::sleep_until(opening_deadline) => return ConnectionOutcome::Fatal(opening::no_response()),
        r = builder.send() => r,
    };
    let mut response = match send_result {
        Ok(r) => r,
        Err(e) => return ConnectionOutcome::TransportError(e),
    };

    // SSE streams MUST return 200. A 2xx-but-not-200 (204 No Content, 206
    // Partial Content, etc.) would have no streamable body and the
    // supervisor would EOF immediately into a reconnect loop without ever
    // surfacing the misclassified status to the consumer. Require exact
    // 200 so unexpected 2xx variants surface as `HttpStatus` with the
    // verbatim status code and the classifier in the outer loop decides
    // whether to retry or fatal them.
    if response.status() != reqwest::StatusCode::OK {
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .map(String::from);
        let retry_after = retry_after::parse_retry_after(response.headers().get("retry-after"));
        let body_result = tokio::select! {
            biased;
            _ = parent_cancel.changed() => return ConnectionOutcome::Cancelled,
            _ = &mut *cancel => return ConnectionOutcome::Cancelled,
            // The status is authoritative even if its diagnostic body stalls.
            () = tokio::time::sleep_until(opening_deadline) => None,
            b = response.bytes() => Some(b),
        };
        let body = match body_result {
            Some(Ok(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            Some(Err(e)) => return ConnectionOutcome::TransportError(e),
            None => String::new(),
        };
        return ConnectionOutcome::HttpStatus {
            status,
            body,
            request_id,
            retry_after,
        };
    }

    if let Err(error) = opening::validate_content_type(response.headers()) {
        return ConnectionOutcome::Fatal(error);
    }
    let mut confirmed = false;

    let mut parser = finesse::Parser::new();
    // Strict gap detection assumes consecutive sequence numbers and
    // fires `ClientError::HistoryGap` on any jump. That contract
    // holds only for UNFILTERED listeners: when a filter is present,
    // the server applies it server-side and silently skips
    // non-matching events, so observed sequences naturally jump from
    // the client's perspective. Relaxed mode logs gaps at DEBUG but
    // does not terminate the watch.
    let mut gap_guard = if request.filter().is_empty() {
        GapGuard::starting_from(wire_from)
    } else {
        GapGuard::relaxed_starting_from(wire_from)
    };

    loop {
        // The heartbeat budget measures time waiting for the next wire
        // chunk, not the supervisor's total iteration time. Wrapping
        // each `response.chunk().await` in a fresh `timeout(budget, ...)`
        // means time spent in `drain_frames` (state-store `put`, channel
        // backpressure inside `send_or_cancel`, parser work) does NOT
        // consume the budget. Otherwise a slow consumer that filled the
        // bounded channel could provoke a false `HeartbeatStarved`
        // reconnect even with a healthy server.
        let timed = tokio::select! {
            biased;
            _ = parent_cancel.changed() => {
                let stop = state.transition(WatchEvent::Stop);
                apply_outcome(last_reconnect_policy, stop);
                return ConnectionOutcome::Cancelled;
            }
            _ = &mut *cancel => {
                let stop = state.transition(WatchEvent::Stop);
                apply_outcome(last_reconnect_policy, stop);
                return ConnectionOutcome::Cancelled;
            }
            () = tokio::time::sleep_until(opening_deadline), if !confirmed => {
                return ConnectionOutcome::Fatal(opening::protocol("Aviso opening deadline exceeded (10s)"));
            }
            r = tokio::time::timeout(budget, response.chunk()) => r,
        };
        let chunk = match timed {
            Ok(c) => c,
            Err(_elapsed) => {
                let starved = state.transition(WatchEvent::HeartbeatStarvation);
                apply_outcome(last_reconnect_policy, starved);
                return ConnectionOutcome::HeartbeatStarved;
            }
        };
        let eof = matches!(chunk, Ok(None));
        // Set when the server sent more for one line or one event than the
        // parser will hold. The frames the parser completed before that
        // point are still delivered below; then the watch ends, since
        // reconnecting would only let the same stream do it again.
        let overflow = match chunk {
            Ok(Some(bytes)) => parser.feed(&bytes).err(),
            Ok(None) => parser.end().err(),
            Err(transport_e) => return ConnectionOutcome::TransportError(transport_e),
        };
        if !confirmed {
            match opening::confirmed(&mut parser, wire_from.is_some()) {
                Ok(true) => {
                    confirmed = true;
                    *retry_counter = 0;
                    apply_outcome(
                        last_reconnect_policy,
                        state.transition(WatchEvent::ConnectionEstablished),
                    );
                    ready.send_replace(true);
                    tracing::debug!(
                        event.name = "client.watch.subscribed",
                        "Aviso stream confirmed"
                    );
                }
                // Keep reading unless the stream has ended or the parser
                // has stopped on an overflow, in which case there is
                // nothing more to wait for and the error below must be
                // reported.
                Ok(false) if !eof && overflow.is_none() => continue,
                Ok(false) => {}
                Err(error) => return ConnectionOutcome::Fatal(error),
            }
        }
        match drain_frames(
            &mut parser,
            request.event_type(),
            state,
            last_reconnect_policy,
            &mut gap_guard,
            commit_cursor,
            pending_commit,
            state_store,
            resume_key,
            request.triggers(),
            trigger_states,
            http,
            tx,
            cancel,
            parent_cancel,
        )
        .await
        {
            // A close frame queued in the same chunk as an overflow does
            // not make the overflow routine: the fatal branch below wins.
            Ok(DrainOutcome::Continue | DrainOutcome::ServerClosed) if overflow.is_some() => {}
            Ok(DrainOutcome::Continue) => {}
            Ok(DrainOutcome::ServerClosed) => return ConnectionOutcome::ServerClosed,
            Ok(DrainOutcome::StopRequested) => return ConnectionOutcome::Cancelled,
            Err(terminal_err) => return ConnectionOutcome::Fatal(terminal_err),
        }
        if let Some(overflow) = overflow {
            let message = overflow.to_string();
            apply_outcome(
                last_reconnect_policy,
                state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                    message.clone(),
                ))),
            );
            return ConnectionOutcome::Fatal(ClientError::StreamProtocol {
                message,
                request_id: None,
            });
        }
        if state.is_terminal() {
            return ConnectionOutcome::ServerClosed;
        }
        if eof {
            // The wire ended without a `connection-closing` frame. The
            // outer reconnect loop classifies this as routine transport
            // loss and reconnects with exponential backoff; long-lived
            // watches survive NAT timeouts and half-open sockets through
            // this path.
            let lost = state.transition(WatchEvent::ConnectionLost {
                reason: ConnectionLossReason::UnexpectedEof,
            });
            apply_outcome(last_reconnect_policy, lost);
            return ConnectionOutcome::UnexpectedEof;
        }
    }
}
