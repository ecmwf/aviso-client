//! Property tests for [`aviso::watch::WatchState`].
//!
//! Generates random sequences of [`WatchEvent`]s and asserts the
//! reducer's spec-correctness invariants for every step. Invariants
//! trace back to ADR D2 (reconnect-as-norm, reconnect classifier,
//! at-least-once delivery) and the oracle review that hardened the
//! reducer surface before implementation. The invariant numbering
//! is local to this file and reused in the commit message that
//! introduced it.
//!
//! The invariants checked here are:
//!
//! 1. `transition()` never panics.
//! 2. Once `is_terminal()` is true, subsequent events preserve the
//!    state and produce `Continue`.
//! 3. `ReplayPhase::Live` is reached from `Replaying` only via
//!    `ReplayCompleted` in `WatchMode::Watch`.
//! 4. In `ReplayOnly`, `ReplayCompleted` never moves out of
//!    `Replaying`.
//! 5. In `ReplayOnly`, `ServerClose(EndOfStream)` terminates with
//!    `CloseReason::EndOfStream` iff the prior phase was
//!    `Replaying { replay_completed: true, .. }`.
//! 6. `WatchEvent::Fatal(_)` and `AuthRefreshCompleted { success:
//!    false }` never produce a `Reconnect` outcome.
//! 7. Once `GapDetected`, the next phase is either `GapDetected`
//!    again or `Closed`; never `Replaying` or `Live`.
//! 8. Every `Reconnect` outcome carries the D2-correct policy for
//!    the event that produced it (combined with mode and prior
//!    phase). Encodes spec rows 9, 10, 11 from the plan.

#![allow(
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: panic-on-invariant-violation is the test diagnostic"
)]

use std::time::Duration;

use aviso::watch::{
    CloseReason, ConnectionLossReason, ConnectionStatus, FatalKind, GapReason, ReconnectPolicy,
    ReplayPhase, ResumeStart, ServerCloseReason, WatchEvent, WatchMode, WatchOutcome, WatchState,
};
use proptest::collection::vec;
use proptest::prelude::*;

fn mode_strategy() -> impl Strategy<Value = WatchMode> {
    prop_oneof![Just(WatchMode::Watch), Just(WatchMode::ReplayOnly)]
}

fn start_strategy() -> impl Strategy<Value = Option<ResumeStart>> {
    prop_oneof![
        Just(None::<ResumeStart>),
        any::<u64>().prop_map(|n| Some(ResumeStart::Sequence(n))),
        "[a-z0-9-]{1,16}".prop_map(|s| Some(ResumeStart::Date(s))),
    ]
}

fn connection_loss_reason_strategy() -> impl Strategy<Value = ConnectionLossReason> {
    prop_oneof![
        Just(ConnectionLossReason::TransportError),
        Just(ConnectionLossReason::UnexpectedEof),
    ]
}

fn server_close_reason_strategy() -> impl Strategy<Value = ServerCloseReason> {
    prop_oneof![
        Just(ServerCloseReason::MaxDurationReached),
        Just(ServerCloseReason::ServerShutdown),
        Just(ServerCloseReason::EndOfStream),
    ]
}

fn gap_reason_strategy() -> impl Strategy<Value = GapReason> {
    prop_oneof![
        (any::<u64>(), any::<u64>()).prop_map(|(o, r)| GapReason::ReplayLimitReached {
            oldest_available: o,
            requested: r
        }),
        (any::<u64>(), any::<u64>()).prop_map(|(e, o)| GapReason::SequenceJump {
            expected: e,
            observed: o
        }),
    ]
}

fn fatal_kind_strategy() -> impl Strategy<Value = FatalKind> {
    prop_oneof![
        Just(FatalKind::MalformedEvent),
        Just(FatalKind::SchemaFingerprintChanged),
        Just(FatalKind::AuthenticationRejectedAfterRefresh),
        Just(FatalKind::TransportRetriesExhausted),
        "[a-z ]{0,32}".prop_map(FatalKind::ProtocolViolation),
    ]
}

fn event_strategy() -> impl Strategy<Value = WatchEvent> {
    prop_oneof![
        Just(WatchEvent::ConnectionEstablished),
        connection_loss_reason_strategy().prop_map(|reason| WatchEvent::ConnectionLost { reason }),
        server_close_reason_strategy().prop_map(|reason| WatchEvent::ServerClose { reason }),
        (0u64..=60_000).prop_map(|ms| WatchEvent::BackoffStarted(Duration::from_millis(ms))),
        Just(WatchEvent::BackoffElapsed),
        Just(WatchEvent::AuthRejected),
        any::<bool>().prop_map(|success| WatchEvent::AuthRefreshCompleted { success }),
        Just(WatchEvent::HeartbeatReceived),
        Just(WatchEvent::HeartbeatStarvation),
        any::<u64>().prop_map(|sequence| WatchEvent::NotificationReceived { sequence }),
        Just(WatchEvent::ReplayCompleted),
        gap_reason_strategy().prop_map(WatchEvent::GapDetected),
        fatal_kind_strategy().prop_map(WatchEvent::Fatal),
        Just(WatchEvent::Stop),
    ]
}

/// The reconnect policy a `Reconnect` outcome must carry for `event`
/// when applied in `mode` with `prior_phase`. Returns `None` for
/// events that should not produce a `Reconnect`.
fn expected_reconnect_policy(
    event: &WatchEvent,
    mode: WatchMode,
    prior_phase: &ReplayPhase,
) -> Option<ReconnectPolicy> {
    match event {
        WatchEvent::ConnectionLost { .. } | WatchEvent::HeartbeatStarvation => {
            Some(ReconnectPolicy::ExponentialBackoff)
        }
        WatchEvent::ServerClose { reason } => match reason {
            ServerCloseReason::MaxDurationReached => Some(ReconnectPolicy::Immediate),
            ServerCloseReason::ServerShutdown => Some(ReconnectPolicy::ShortBackoff),
            ServerCloseReason::EndOfStream => match mode {
                WatchMode::Watch => Some(ReconnectPolicy::Immediate),
                WatchMode::ReplayOnly => {
                    if matches!(
                        prior_phase,
                        ReplayPhase::Replaying {
                            replay_completed: true,
                            ..
                        }
                    ) {
                        None
                    } else {
                        Some(ReconnectPolicy::Immediate)
                    }
                }
                _ => panic!("unexpected WatchMode in test expectations: {mode:?}"),
            },
            _ => panic!("unexpected ServerCloseReason in test expectations: {reason:?}"),
        },
        _ => None,
    }
}

proptest! {
    #[test]
    fn invariants_hold_for_random_event_sequences(
        mode in mode_strategy(),
        start in start_strategy(),
        events in vec(event_strategy(), 0..=64),
    ) {
        let mut state = WatchState::new(mode, start);
        let mut terminal_snapshot: Option<WatchState> = None;

        for event in events {
            let prior = state.clone();
            let outcome = state.transition(event.clone());

            // Invariant 2: terminal is sticky.
            if let Some(snapshot) = &terminal_snapshot {
                prop_assert_eq!(
                    &state,
                    snapshot,
                    "state mutated after terminal: event={:?}",
                    event
                );
                prop_assert_eq!(
                    outcome.clone(),
                    WatchOutcome::Continue,
                    "non-Continue outcome after terminal: event={:?}",
                    event
                );
                continue;
            }

            // Invariant 3: Live is reached from Replaying only via
            // ReplayCompleted in Watch mode.
            if matches!(state.replay_phase(), ReplayPhase::Live)
                && !matches!(prior.replay_phase(), ReplayPhase::Live)
            {
                let legal_live_entry = mode == WatchMode::Watch
                    && matches!(event, WatchEvent::ReplayCompleted)
                    && matches!(prior.replay_phase(), ReplayPhase::Replaying { .. });
                prop_assert!(
                    legal_live_entry,
                    "illegal Live transition: prior={:?}, event={:?}, mode={:?}",
                    prior, event, mode
                );
            }

            // Invariant 4: in ReplayOnly, ReplayCompleted from Replaying
            // never leaves Replaying.
            if mode == WatchMode::ReplayOnly
                && matches!(event, WatchEvent::ReplayCompleted)
                && matches!(prior.replay_phase(), ReplayPhase::Replaying { .. })
            {
                prop_assert!(
                    matches!(state.replay_phase(), ReplayPhase::Replaying { .. }),
                    "ReplayCompleted left Replaying in ReplayOnly: after={:?}",
                    state
                );
            }

            // Invariant 5: ReplayOnly + EndOfStream terminates iff
            // replay_completed was already true.
            if mode == WatchMode::ReplayOnly
                && matches!(
                    event,
                    WatchEvent::ServerClose {
                        reason: ServerCloseReason::EndOfStream
                    }
                )
            {
                let prior_rc = matches!(
                    prior.replay_phase(),
                    ReplayPhase::Replaying {
                        replay_completed: true,
                        ..
                    }
                );
                let terminated_eos = matches!(
                    state.replay_phase(),
                    ReplayPhase::Closed {
                        reason: CloseReason::EndOfStream
                    }
                );
                prop_assert_eq!(
                    prior_rc,
                    terminated_eos,
                    "ReplayOnly EndOfStream termination mismatch: prior={:?}, after={:?}",
                    prior,
                    state
                );
            }

            // Invariant 6: fatal-class events never produce Reconnect.
            match &event {
                WatchEvent::Fatal(_)
                | WatchEvent::AuthRefreshCompleted { success: false } => {
                    prop_assert!(
                        !matches!(outcome, WatchOutcome::Reconnect { .. }),
                        "fatal-class event produced Reconnect: {:?} -> {:?}",
                        event,
                        outcome
                    );
                }
                _ => {}
            }

            // Invariant 7: GapDetected phase escapes only to Closed (or
            // stays GapDetected).
            if matches!(prior.replay_phase(), ReplayPhase::GapDetected { .. }) {
                let stayed_or_closed = matches!(
                    state.replay_phase(),
                    ReplayPhase::GapDetected { .. } | ReplayPhase::Closed { .. }
                );
                prop_assert!(
                    stayed_or_closed,
                    "GapDetected escaped to non-Closed phase: prior={:?}, event={:?}, after={:?}",
                    prior,
                    event,
                    state
                );
            }

            // Invariant 8: Reconnect outcomes carry the spec-mandated
            // policy for the event that produced them.
            if let WatchOutcome::Reconnect { policy } = outcome {
                let expected = expected_reconnect_policy(&event, mode, prior.replay_phase());
                prop_assert_eq!(
                    Some(policy),
                    expected,
                    "Reconnect policy mismatch: event={:?}, mode={:?}, prior_phase={:?}",
                    event,
                    mode,
                    prior.replay_phase()
                );
            }

            // Capture the snapshot on the first transition to terminal so
            // invariant 2 can compare against it.
            if state.is_terminal() && terminal_snapshot.is_none() {
                terminal_snapshot = Some(state.clone());
            }
        }
    }

    #[test]
    fn connection_status_stays_within_expected_set(
        mode in mode_strategy(),
        start in start_strategy(),
        events in vec(event_strategy(), 0..=64),
    ) {
        // Invariant: connection_status is always one of the four
        // defined variants. This is structurally guaranteed by the
        // enum, but the test exercises every transition for it.
        let mut state = WatchState::new(mode, start);
        for event in events {
            let _ = state.transition(event);
            let status = state.connection_status().clone();
            prop_assert!(matches!(
                status,
                ConnectionStatus::Connected
                    | ConnectionStatus::Reconnecting
                    | ConnectionStatus::BackoffWait(_)
                    | ConnectionStatus::RefreshingAuth
            ));
        }
    }
}
