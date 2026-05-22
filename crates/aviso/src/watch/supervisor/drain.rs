use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, watch};

use super::{DrainOutcome, GapGuard, PendingCommit, apply_outcome, send_or_cancel};
use crate::state::{Checkpoint, ResumeKey, StateStore};
use crate::watch::wire::{
    WireCloudEvent, WireConnectionClosing, WireConnectionEstablished, WireErrorEvent,
    WireReplayControl,
};
use crate::watch::{
    FatalKind, GapReason, ReconnectPolicy, ServerCloseReason, WatchEvent, WatchState,
};
use crate::{ClientError, Notification, parse_cloudevent_id};

/// Drain every frame currently queued in the SSE parser and feed it
/// through the supervisor's mapping table.
///
/// Returns `Ok(DrainOutcome)`:
/// - `Continue`: routine frame handling; caller keeps reading from the
///   wire.
/// - `ServerClosed`: a `connection-closing` frame was processed; the
///   reducer's `WatchEvent::ServerClose` transition has run and the
///   caller should stop reading from this connection.
/// - `StopRequested`: per-stream or parent-drop cancellation observed
///   during a `send_or_cancel`, or the consumer dropped the receiver;
///   the caller should exit without surfacing anything.
///
/// Returns `Err(ClientError)` only for terminal wire-level conditions
/// that the consumer must see as a typed error item on the stream: a
/// malformed `CloudEvent` id, a server `error` event, an unknown
/// `connection-closing.reason`, a JSON decode failure, or a
/// state-store persistence failure.
#[allow(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "the SSE-event-type dispatch is a flat match over six wire variants; splitting each branch into its own helper trades readability for line count, and reviewers want to see the full mapping table in one place"
)]
pub(super) async fn drain_frames(
    parser: &mut finesse::Parser,
    state: &mut WatchState,
    last_reconnect_policy: &mut Option<ReconnectPolicy>,
    gap_guard: &mut GapGuard,
    commit_cursor: &mut Option<u64>,
    pending_commit: &mut Option<PendingCommit>,
    state_store: Option<&Arc<dyn StateStore>>,
    resume_key: &ResumeKey,
    triggers: &[crate::watch::Trigger],
    trigger_states: &mut [crate::watch::trigger::TriggerState],
    http: &reqwest::Client,
    tx: &mpsc::Sender<Result<Notification, ClientError>>,
    cancel: &mut oneshot::Receiver<()>,
    parent_cancel: &mut watch::Receiver<bool>,
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
                    apply_outcome(
                        last_reconnect_policy,
                        state.transition(WatchEvent::ConnectionEstablished),
                    );
                    continue;
                }
                let wire: WireCloudEvent = serde_json::from_value(raw)?;
                let (event_type, sequence) = match parse_cloudevent_id(&wire.id) {
                    Ok(v) => v,
                    Err(e) => {
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::Fatal(FatalKind::MalformedEvent)),
                        );
                        return Err(e);
                    }
                };
                apply_outcome(
                    last_reconnect_policy,
                    state.transition(WatchEvent::NotificationReceived { sequence }),
                );
                match gap_guard.observe(sequence) {
                    Ok(()) => {
                        let notification = Notification {
                            event_type: event_type.clone(),
                            sequence,
                            identifier: wire.data.identifier,
                            payload: wire.data.payload,
                        };
                        // Commit-on-next-send: persist the *previous* notification
                        // before sending the current one, so pulling N implies the
                        // previous send (item N-1) is durable. Always advance the
                        // in-memory `commit_cursor` to keep reconnect-from-current
                        // working without a store. If a store is configured and
                        // its `put` fails, surface a terminal error.
                        if let Some(prev) = pending_commit.as_ref() {
                            if let Some(store) = state_store {
                                let checkpoint =
                                    Checkpoint::new(prev.sequence, Some(prev.event_id.clone()));
                                // CANCELLATION SAFETY: this `put` is
                                // INTENTIONALLY NOT raced against cancel
                                // arms. The supervisor's contract is "an
                                // in-progress store write is allowed to
                                // complete so the underlying file (or
                                // future durable backend) is never left
                                // half-written". Cancellation that
                                // arrives DURING the put extends exit
                                // latency by the put's duration; the
                                // next `send_or_cancel` then observes
                                // the cancel and the supervisor exits.
                                // Cancellation that arrives between the
                                // preceding chunk-read and this put is
                                // NOT observed here (there is no cancel
                                // check between chunk decoding and this
                                // put); the put runs anyway, which is
                                // harmless because puts are idempotent
                                // and the next process resumes from
                                // exactly the same cursor. Cancellation
                                // that arrived earlier was observed at
                                // the preceding chunk-read select and
                                // this code path is never reached.
                                // The latency trade-off (typically tens
                                // of milliseconds for `JsonFileStore`'s
                                // `fsync` on local disk) is documented
                                // on the `StateStore` trait so custom
                                // implementations know to keep `put`
                                // bounded.
                                if let Err(e) = store.put(resume_key, checkpoint).await {
                                    return Err(ClientError::from(e));
                                }
                            }
                            *commit_cursor = Some(prev.sequence);
                        }
                        // Trigger pipeline runs BEFORE the channel send so a
                        // required-trigger failure terminates the watch
                        // without ever advancing `pending_commit` for this
                        // notification; the cursor stays at the previous
                        // value and restart re-delivers this N. The pipeline
                        // itself is cancel-aware between triggers and
                        // between retry backoffs but lets a single dispatch
                        // attempt run to completion (same atomicity contract
                        // as `state_store.put`).
                        match crate::watch::trigger::dispatch_triggers(
                            triggers,
                            trigger_states,
                            &notification,
                            parent_cancel,
                            cancel,
                            http,
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(crate::watch::trigger::DispatchOutcome::Cancelled) => {
                                return Ok(DrainOutcome::StopRequested);
                            }
                            Err(crate::watch::trigger::DispatchOutcome::RequiredFailed {
                                kind,
                                source,
                            }) => {
                                let err = ClientError::TriggerFailed { kind, source };
                                let _ = send_or_cancel(tx, Err(err), cancel, parent_cancel).await;
                                return Ok(DrainOutcome::StopRequested);
                            }
                        }
                        if send_or_cancel(tx, Ok(notification), cancel, parent_cancel)
                            .await
                            .is_err()
                        {
                            return Ok(DrainOutcome::StopRequested);
                        }
                        *pending_commit = Some(PendingCommit {
                            sequence,
                            event_id: format!("{event_type}@{sequence}"),
                        });
                    }
                    Err(reason) => {
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::GapDetected(reason)),
                        );
                        let _ = send_or_cancel(
                            tx,
                            Err(ClientError::HistoryGap { reason }),
                            cancel,
                            parent_cancel,
                        )
                        .await;
                        return Ok(DrainOutcome::StopRequested);
                    }
                }
            }
            "heartbeat" => {
                apply_outcome(
                    last_reconnect_policy,
                    state.transition(WatchEvent::HeartbeatReceived),
                );
            }
            "replay-control" => {
                let wire: WireReplayControl = serde_json::from_str(&msg.data)?;
                match wire.tag.as_str() {
                    "replay_completed" => {
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::ReplayCompleted),
                        );
                    }
                    "notification_replay_limit_reached" => {
                        let Some(max_allowed) = wire.max_allowed else {
                            // The server's payload for this control event is
                            // documented to carry `max_allowed`. Silently
                            // defaulting a missing value would publish a
                            // misleading `ReplayLimitReached { max_allowed: 0 }`
                            // and hide a server protocol regression; surface a
                            // typed protocol error instead.
                            let message =
                                "replay-control: notification_replay_limit_reached missing max_allowed"
                                    .to_string();
                            apply_outcome(
                                last_reconnect_policy,
                                state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                                    message.clone(),
                                ))),
                            );
                            return Err(ClientError::StreamProtocol {
                                message,
                                request_id: None,
                            });
                        };
                        let reason = GapReason::ReplayLimitReached { max_allowed };
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::GapDetected(reason)),
                        );
                        let _ = send_or_cancel(
                            tx,
                            Err(ClientError::HistoryGap { reason }),
                            cancel,
                            parent_cancel,
                        )
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
                        apply_outcome(
                            last_reconnect_policy,
                            state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                                message.clone(),
                            ))),
                        );
                        return Err(ClientError::StreamProtocol {
                            message,
                            request_id: wire.request_id,
                        });
                    }
                };
                apply_outcome(
                    last_reconnect_policy,
                    state.transition(WatchEvent::ServerClose { reason }),
                );
                // Every recognised `connection-closing` reason ends this
                // connection. The outer reconnect loop reads the reducer's
                // post-transition state to decide whether to reconnect; in
                // watch mode the routine close reasons all reconnect, in
                // replay-only mode `end_of_stream` after `replay_completed`
                // terminates naturally.
                return Ok(DrainOutcome::ServerClosed);
            }
            "error" => {
                let wire: WireErrorEvent = serde_json::from_str(&msg.data)?;
                let message = wire
                    .message
                    .clone()
                    .or_else(|| wire.error.clone())
                    .unwrap_or_else(|| "server error event".to_string());
                apply_outcome(
                    last_reconnect_policy,
                    state.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
                        message.clone(),
                    ))),
                );
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
