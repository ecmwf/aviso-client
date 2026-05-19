use std::time::Duration;

use super::*;
use crate::watch::{ConnectionLossReason, ServerCloseReason, WatchEvent};

#[test]
fn constructor_without_start_enters_live() {
    let s = WatchState::watch(None);
    assert_eq!(s.replay_phase(), &ReplayPhase::Live);
    assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
    assert_eq!(s.mode(), WatchMode::Watch);
    assert!(!s.is_terminal());
}

#[test]
fn constructor_with_sequence_enters_replaying() {
    let s = WatchState::watch(Some(ResumeStart::AfterSequence(42)));
    assert_eq!(
        s.replay_phase(),
        &ReplayPhase::Replaying {
            start: ResumeStart::AfterSequence(42),
            replay_completed: false,
        }
    );
}

#[test]
fn constructor_with_date_enters_replaying() {
    let s = WatchState::replay_only(ResumeStart::Date("2026-01-01".into()));
    assert_eq!(
        s.replay_phase(),
        &ReplayPhase::Replaying {
            start: ResumeStart::Date("2026-01-01".into()),
            replay_completed: false,
        }
    );
}

#[test]
fn connection_established_moves_to_connected() {
    let mut s = WatchState::watch(None);
    let out = s.transition(WatchEvent::ConnectionEstablished);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s.connection_status(), &ConnectionStatus::Connected);
}

#[test]
fn transport_error_reconnects_with_exponential_backoff() {
    let mut s = WatchState::watch(None);
    let _ = s.transition(WatchEvent::ConnectionEstablished);
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
fn server_max_duration_reconnects_immediately() {
    let mut s = WatchState::watch(None);
    let _ = s.transition(WatchEvent::ConnectionEstablished);
    let out = s.transition(WatchEvent::ServerClose {
        reason: ServerCloseReason::MaxDurationReached,
    });
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::Immediate
        }
    );
}

#[test]
fn server_shutdown_reconnects_with_short_backoff() {
    let mut s = WatchState::watch(None);
    let out = s.transition(WatchEvent::ServerClose {
        reason: ServerCloseReason::ServerShutdown,
    });
    assert_eq!(
        out,
        WatchOutcome::Reconnect {
            policy: ReconnectPolicy::ShortBackoff
        }
    );
}

#[test]
fn replay_completed_in_watch_moves_to_live() {
    let mut s = WatchState::watch(Some(ResumeStart::AfterSequence(1)));
    let out = s.transition(WatchEvent::ReplayCompleted);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(s.replay_phase(), &ReplayPhase::Live);
}

#[test]
fn replay_completed_in_replay_only_flips_flag_without_leaving_replaying() {
    let mut s = WatchState::replay_only(ResumeStart::AfterSequence(1));
    let out = s.transition(WatchEvent::ReplayCompleted);
    assert_eq!(out, WatchOutcome::Continue);
    assert_eq!(
        s.replay_phase(),
        &ReplayPhase::Replaying {
            start: ResumeStart::AfterSequence(1),
            replay_completed: true,
        }
    );
}

#[test]
fn end_of_stream_in_replay_only_after_replay_completed_terminates() {
    let mut s = WatchState::replay_only(ResumeStart::AfterSequence(1));
    let _ = s.transition(WatchEvent::ReplayCompleted);
    let out = s.transition(WatchEvent::ServerClose {
        reason: ServerCloseReason::EndOfStream,
    });
    assert_eq!(
        out,
        WatchOutcome::Stop {
            reason: CloseReason::EndOfStream,
        }
    );
    assert!(s.is_terminal());
}

#[test]
fn stop_event_terminates_with_user_requested() {
    let mut s = WatchState::watch(None);
    let out = s.transition(WatchEvent::Stop);
    assert_eq!(
        out,
        WatchOutcome::Stop {
            reason: CloseReason::UserRequested,
        }
    );
    assert!(s.is_terminal());
}

#[test]
fn fatal_event_terminates_with_fatal_close_reason() {
    let mut s = WatchState::watch(None);
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
fn auth_rejected_emits_refresh_auth_outcome() {
    let mut s = WatchState::watch(None);
    let _ = s.transition(WatchEvent::ConnectionEstablished);
    let out = s.transition(WatchEvent::AuthRejected);
    assert_eq!(out, WatchOutcome::RefreshAuth);
    assert_eq!(s.connection_status(), &ConnectionStatus::RefreshingAuth);
}

#[test]
fn auth_refresh_failure_terminates_with_specific_fatal_kind() {
    let mut s = WatchState::watch(None);
    let _ = s.transition(WatchEvent::AuthRejected);
    let out = s.transition(WatchEvent::AuthRefreshCompleted { success: false });
    assert_eq!(
        out,
        WatchOutcome::Stop {
            reason: CloseReason::Fatal {
                kind: FatalKind::AuthenticationRejectedAfterRefresh,
            },
        }
    );
    assert!(s.is_terminal());
}

#[test]
fn gap_detected_emits_gap_outcome_and_enters_gap_phase() {
    let mut s = WatchState::watch(None);
    let reason = GapReason::ReplayLimitReached { max_allowed: 100 };
    let out = s.transition(WatchEvent::GapDetected(reason));
    assert_eq!(out, WatchOutcome::Gap { reason });
    assert_eq!(s.replay_phase(), &ReplayPhase::GapDetected { reason });
}

#[test]
fn backoff_started_stores_duration_then_elapsed_returns_to_reconnecting() {
    let mut s = WatchState::watch(None);
    let d = Duration::from_millis(500);
    let _ = s.transition(WatchEvent::BackoffStarted(d));
    assert_eq!(s.connection_status(), &ConnectionStatus::BackoffWait(d));
    let _ = s.transition(WatchEvent::BackoffElapsed);
    assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
}

#[test]
fn terminal_state_swallows_all_subsequent_events() {
    let mut s = WatchState::watch(None);
    let _ = s.transition(WatchEvent::Stop);
    let snapshot = s.clone();
    // Every event becomes a no-op returning Continue and leaves state
    // unchanged.
    for event in [
        WatchEvent::ConnectionEstablished,
        WatchEvent::ConnectionLost {
            reason: ConnectionLossReason::TransportError,
        },
        WatchEvent::ServerClose {
            reason: ServerCloseReason::MaxDurationReached,
        },
        WatchEvent::BackoffStarted(Duration::from_millis(1)),
        WatchEvent::BackoffElapsed,
        WatchEvent::AuthRejected,
        WatchEvent::AuthRefreshCompleted { success: true },
        WatchEvent::HeartbeatReceived,
        WatchEvent::HeartbeatStarvation,
        WatchEvent::NotificationReceived { sequence: 1 },
        WatchEvent::ReplayCompleted,
        WatchEvent::GapDetected(GapReason::SequenceJump {
            expected: 1,
            observed: 3,
        }),
        WatchEvent::Fatal(FatalKind::MalformedEvent),
        WatchEvent::Stop,
    ] {
        let out = s.transition(event);
        assert_eq!(out, WatchOutcome::Continue);
        assert_eq!(s, snapshot);
    }
}
