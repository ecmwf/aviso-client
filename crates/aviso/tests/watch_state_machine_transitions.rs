//! Table-driven transition coverage for [`aviso::watch::WatchState`].
//!
//! Each test corresponds to one row of the canonical transition table
//! specified by D2 (and reproduced in the commit message that
//! introduced `crates/aviso/src/watch/state.rs`). The basic happy
//! paths are covered by unit tests inside that file; this file
//! exercises mode-specific branches, sub-cases (rows 6a-d, 15a-d),
//! and idempotence/no-op paths that the unit tests omit.
//!
//! Tests use the public surface only (D15: fields are private; tests
//! drive the state machine through events, not field mutation).

#![allow(
    clippy::panic,
    reason = "test code: panic-on-unexpected variant is the standard test diagnostic"
)]

use std::time::Duration;

use aviso::watch::{
    CloseReason, ConnectionLossReason, ConnectionStatus, FatalKind, GapReason, ReconnectPolicy,
    ReplayPhase, ResumeStart, ServerCloseReason, WatchEvent, WatchMode, WatchOutcome, WatchState,
};

/// Build a fresh `Watch`-mode state, drive it to `Connected`, return
/// it. Useful for tests that start from a "live and connected" base.
fn watch_live_connected() -> WatchState {
    let mut s = WatchState::new(WatchMode::Watch, None);
    let _ = s.transition(WatchEvent::ConnectionEstablished);
    s
}

/// Build a fresh `ReplayOnly` state at sequence 1, drive it to
/// `Connected`. The replay phase remains `Replaying { rc: false }`.
fn replay_only_connected() -> WatchState {
    let mut s = WatchState::new(WatchMode::ReplayOnly, Some(ResumeStart::Sequence(1)));
    let _ = s.transition(WatchEvent::ConnectionEstablished);
    s
}

// ---------------------------------------------------------------------------
// Row 2 and 3: ConnectionLost variants both reconnect with exponential backoff.
// ---------------------------------------------------------------------------

#[test]
fn row_2_connection_lost_transport_error_reconnects_with_exponential_backoff() {
    let mut s = watch_live_connected();
    let out = s.transition(WatchEvent::ConnectionLost {
        reason: ConnectionLossReason::TransportError,
    });
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::ExponentialBackoff
        }
    );
    assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
}

#[test]
fn row_3_connection_lost_unexpected_eof_reconnects_with_exponential_backoff() {
    let mut s = watch_live_connected();
    let out = s.transition(WatchEvent::ConnectionLost {
        reason: ConnectionLossReason::UnexpectedEof,
    });
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::ExponentialBackoff
        }
    );
    assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
}

// ---------------------------------------------------------------------------
// Row 6a / 6b / 6c / 6d: ServerClose(EndOfStream) per mode + replay phase.
// ---------------------------------------------------------------------------

#[test]
fn row_6a_end_of_stream_in_watch_mode_reconnects_immediately() {
    let mut s = watch_live_connected();
    let out = s.transition(WatchEvent::ServerClose {
        reason: ServerCloseReason::EndOfStream,
    });
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::Immediate
        }
    );
    assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
    assert_eq!(s.replay_phase(), &ReplayPhase::Live);
}

#[test]
fn row_6a_end_of_stream_in_watch_mode_while_replaying_reconnects_immediately() {
    let mut s = WatchState::new(WatchMode::Watch, Some(ResumeStart::Sequence(5)));
    let _ = s.transition(WatchEvent::ConnectionEstablished);
    let out = s.transition(WatchEvent::ServerClose {
        reason: ServerCloseReason::EndOfStream,
    });
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::Immediate
        }
    );
    // Replay phase unchanged: still Replaying.
    assert_eq!(
        s.replay_phase(),
        &ReplayPhase::Replaying {
            start: ResumeStart::Sequence(5),
            replay_completed: false,
        }
    );
}

#[test]
fn row_6c_end_of_stream_in_replay_only_before_replay_completed_reconnects_immediately() {
    let mut s = replay_only_connected();
    let out = s.transition(WatchEvent::ServerClose {
        reason: ServerCloseReason::EndOfStream,
    });
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::Immediate
        }
    );
    assert!(!s.is_terminal());
}

#[test]
fn row_6d_end_of_stream_in_replay_only_from_gap_phase_reconnects_immediately() {
    // Edge case: ReplayOnly with non-Replaying phase (only reachable via
    // GapDetected since Live is unreachable in ReplayOnly). Reducer
    // reconnects rather than terminating, matching the spec.
    let mut s = replay_only_connected();
    let _ = s.transition(WatchEvent::GapDetected(GapReason::SequenceJump {
        expected: 1,
        observed: 5,
    }));
    let out = s.transition(WatchEvent::ServerClose {
        reason: ServerCloseReason::EndOfStream,
    });
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::Immediate
        }
    );
    assert!(!s.is_terminal());
}

// ---------------------------------------------------------------------------
// Row 8b: BackoffElapsed from non-BackoffWait status is a no-op.
// ---------------------------------------------------------------------------

#[test]
fn row_8b_backoff_elapsed_from_connected_status_is_a_noop() {
    let mut s = watch_live_connected();
    let before = s.connection_status().clone();
    let out = s.transition(WatchEvent::BackoffElapsed);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s.connection_status(), &before);
}

#[test]
fn row_8b_backoff_elapsed_from_refreshing_auth_is_a_noop() {
    let mut s = watch_live_connected();
    let _ = s.transition(WatchEvent::AuthRejected);
    assert_eq!(s.connection_status(), &ConnectionStatus::RefreshingAuth);
    let out = s.transition(WatchEvent::BackoffElapsed);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s.connection_status(), &ConnectionStatus::RefreshingAuth);
}

// ---------------------------------------------------------------------------
// Row 9 / 10: Auth refresh round trip restores Reconnecting on success.
// ---------------------------------------------------------------------------

#[test]
fn row_10_auth_refresh_success_returns_to_reconnecting() {
    let mut s = watch_live_connected();
    let _ = s.transition(WatchEvent::AuthRejected);
    let out = s.transition(WatchEvent::AuthRefreshCompleted { success: true });
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
}

// ---------------------------------------------------------------------------
// Rows 12 and 14: heartbeats and notifications are pure observations.
// ---------------------------------------------------------------------------

#[test]
fn row_12_heartbeat_received_leaves_state_unchanged() {
    let mut s = watch_live_connected();
    let before = s.clone();
    let out = s.transition(WatchEvent::HeartbeatReceived);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s, before);
}

#[test]
fn row_14_notification_received_leaves_state_unchanged() {
    let mut s = watch_live_connected();
    let before = s.clone();
    let out = s.transition(WatchEvent::NotificationReceived { sequence: 42 });
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s, before);
}

// ---------------------------------------------------------------------------
// Row 15c / 15d: ReplayCompleted idempotence outside the `Replaying{rc=false}`
// case.
// ---------------------------------------------------------------------------

#[test]
fn row_15c_replay_completed_when_already_completed_in_replay_only_is_idempotent() {
    let mut s = replay_only_connected();
    let _ = s.transition(WatchEvent::ReplayCompleted);
    let after_first = s.clone();
    let out = s.transition(WatchEvent::ReplayCompleted);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s, after_first);
}

#[test]
fn row_15d_replay_completed_in_live_phase_is_idempotent() {
    let mut s = watch_live_connected();
    assert_eq!(s.replay_phase(), &ReplayPhase::Live);
    let before = s.clone();
    let out = s.transition(WatchEvent::ReplayCompleted);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s, before);
}

#[test]
fn row_15d_replay_completed_in_gap_phase_is_idempotent() {
    let mut s = watch_live_connected();
    let _ = s.transition(WatchEvent::GapDetected(GapReason::ReplayLimitReached {
        oldest_available: 100,
        requested: 1,
    }));
    let before = s.clone();
    let out = s.transition(WatchEvent::ReplayCompleted);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s, before);
}

// ---------------------------------------------------------------------------
// Row 17: Fatal event variants each terminate with the matching kind.
// ---------------------------------------------------------------------------

#[test]
fn row_17_fatal_malformed_event_terminates() {
    let mut s = watch_live_connected();
    let out = s.transition(WatchEvent::Fatal(FatalKind::MalformedEvent));
    assert_eq!(
        out,
        WatchOutcome::Stop {
            reason: CloseReason::Fatal {
                kind: FatalKind::MalformedEvent,
            },
        }
    );
    assert!(s.is_terminal());
}

#[test]
fn row_17_fatal_schema_fingerprint_changed_terminates() {
    let mut s = watch_live_connected();
    let out = s.transition(WatchEvent::Fatal(FatalKind::SchemaFingerprintChanged));
    assert_eq!(
        out,
        WatchOutcome::Stop {
            reason: CloseReason::Fatal {
                kind: FatalKind::SchemaFingerprintChanged,
            },
        }
    );
    assert!(s.is_terminal());
}

#[test]
fn row_17_fatal_transport_retries_exhausted_terminates() {
    let mut s = watch_live_connected();
    let out = s.transition(WatchEvent::Fatal(FatalKind::TransportRetriesExhausted));
    assert_eq!(
        out,
        WatchOutcome::Stop {
            reason: CloseReason::Fatal {
                kind: FatalKind::TransportRetriesExhausted,
            },
        }
    );
    assert!(s.is_terminal());
}

#[test]
fn row_17_fatal_protocol_violation_carries_description() {
    let mut s = watch_live_connected();
    let out = s.transition(WatchEvent::Fatal(FatalKind::ProtocolViolation(
        "unknown SSE event type 'fizz'".to_string(),
    )));
    assert_eq!(
        out,
        WatchOutcome::Stop {
            reason: CloseReason::Fatal {
                kind: FatalKind::ProtocolViolation("unknown SSE event type 'fizz'".to_string()),
            },
        }
    );
    assert!(s.is_terminal());
}

// ---------------------------------------------------------------------------
// Row 13: HeartbeatStarvation in its own right (commit 2 only exercises it
// inside the terminal-sticky test).
// ---------------------------------------------------------------------------

#[test]
fn row_13_heartbeat_starvation_reconnects_with_exponential_backoff() {
    let mut s = watch_live_connected();
    let out = s.transition(WatchEvent::HeartbeatStarvation);
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::ExponentialBackoff
        }
    );
    assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
}

// ---------------------------------------------------------------------------
// Gap reason variants: ReplayLimitReached and SequenceJump each surface
// through the Gap outcome.
// ---------------------------------------------------------------------------

#[test]
fn gap_replay_limit_reached_is_surfaced_in_outcome() {
    let mut s = watch_live_connected();
    let reason = GapReason::ReplayLimitReached {
        oldest_available: 200,
        requested: 50,
    };
    let out = s.transition(WatchEvent::GapDetected(reason));
    assert_eq!(out, WatchOutcome::Gap { reason });
}

#[test]
fn gap_sequence_jump_is_surfaced_in_outcome() {
    let mut s = watch_live_connected();
    let reason = GapReason::SequenceJump {
        expected: 10,
        observed: 15,
    };
    let out = s.transition(WatchEvent::GapDetected(reason));
    assert_eq!(out, WatchOutcome::Gap { reason });
}

// ---------------------------------------------------------------------------
// Mode invariant: in ReplayOnly, the reducer never produces ReplayPhase::Live
// through legal events.
// ---------------------------------------------------------------------------

#[test]
fn replay_only_replay_completed_never_enters_live_phase() {
    let mut s = WatchState::new(WatchMode::ReplayOnly, Some(ResumeStart::Sequence(1)));
    let _ = s.transition(WatchEvent::ConnectionEstablished);
    let _ = s.transition(WatchEvent::ReplayCompleted);
    assert_ne!(s.replay_phase(), &ReplayPhase::Live);
    match s.replay_phase() {
        ReplayPhase::Replaying {
            replay_completed, ..
        } => assert!(replay_completed),
        other => panic!("expected Replaying{{rc=true}}, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Connection-status orthogonality: a successful auth refresh does not change
// the replay phase, regardless of the phase the supervisor was in.
// ---------------------------------------------------------------------------

#[test]
fn auth_refresh_round_trip_does_not_change_replay_phase() {
    let mut s = WatchState::new(WatchMode::Watch, Some(ResumeStart::Sequence(1)));
    let _ = s.transition(WatchEvent::ConnectionEstablished);
    let phase_before = s.replay_phase().clone();
    let _ = s.transition(WatchEvent::AuthRejected);
    let _ = s.transition(WatchEvent::AuthRefreshCompleted { success: true });
    assert_eq!(s.replay_phase(), &phase_before);
}

// ---------------------------------------------------------------------------
// BackoffStarted carries an arbitrary Duration verbatim.
// ---------------------------------------------------------------------------

#[test]
fn backoff_started_stores_arbitrary_duration() {
    let mut s = watch_live_connected();
    let d = Duration::from_secs(7);
    let _ = s.transition(WatchEvent::BackoffStarted(d));
    assert_eq!(s.connection_status(), &ConnectionStatus::BackoffWait(d));
}
