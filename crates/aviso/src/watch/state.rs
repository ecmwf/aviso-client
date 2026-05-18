//! [`WatchState`]: the orthogonal-product reducer for the watch session.
//!
//! The reducer implements the transition rules specified by D2 (the
//! reconnect classifier and state-machine sketch in
//! `docs/src/internals/decisions.md`). Each match arm carries a row
//! number; the row numbers match the canonical transition table in
//! the commit message that introduced this file, kept here as a
//! traceable audit trail for spec conformance. D15 constrains the
//! public surface: fields are private and callers use the accessor
//! methods.

use super::{
    CloseReason, ConnectionStatus, FatalKind, GapReason, ReconnectPolicy, ReplayPhase, ResumeStart,
    ServerCloseReason, WatchEvent, WatchMode, WatchOutcome,
};

/// Watch session state machine.
///
/// `WatchState` is the orthogonal product of [`ReplayPhase`] x
/// [`ConnectionStatus`], plus the immutable [`WatchMode`] picked at
/// construction time. Callers advance the state by feeding
/// [`WatchEvent`]s through [`Self::transition`], which returns a
/// [`WatchOutcome`] telling the supervisor what to do next.
///
/// The reducer is sync, push-based, and owns no resources. It does not
/// track checkpoint state (see D17); the supervisor handles cursor
/// bookkeeping in response to [`WatchEvent::NotificationReceived`] and
/// trigger completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchState {
    replay_phase: ReplayPhase,
    connection_status: ConnectionStatus,
    mode: WatchMode,
}

impl WatchState {
    /// Construct a watch state for `mode`, optionally with a resume
    /// position.
    ///
    /// `start = None` enters [`ReplayPhase::Live`] directly (no replay
    /// to do). `start = Some(_)` enters
    /// [`ReplayPhase::Replaying`] with `replay_completed: false`. In
    /// both cases [`ConnectionStatus`] starts as `Reconnecting`
    /// because no transport is open yet.
    #[must_use]
    pub fn new(mode: WatchMode, start: Option<ResumeStart>) -> Self {
        let replay_phase = match start {
            Some(start) => ReplayPhase::Replaying {
                start,
                replay_completed: false,
            },
            None => ReplayPhase::Live,
        };
        Self {
            replay_phase,
            connection_status: ConnectionStatus::Reconnecting,
            mode,
        }
    }

    /// Borrow the current replay phase.
    #[must_use]
    pub fn replay_phase(&self) -> &ReplayPhase {
        &self.replay_phase
    }

    /// Borrow the current connection status.
    #[must_use]
    pub fn connection_status(&self) -> &ConnectionStatus {
        &self.connection_status
    }

    /// Return the session's mode.
    #[must_use]
    pub fn mode(&self) -> WatchMode {
        self.mode
    }

    /// `true` iff the session has terminated.
    ///
    /// Once terminal, [`Self::transition`] becomes a no-op returning
    /// [`WatchOutcome::Continue`]; the supervisor reads this flag to
    /// decide whether to tear down.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self.replay_phase, ReplayPhase::Closed { .. })
    }

    /// Advance the state by `event`.
    ///
    /// Returns the outcome the supervisor must act on. Once terminal
    /// (see [`Self::is_terminal`]), every event is a no-op returning
    /// [`WatchOutcome::Continue`]; the state is preserved.
    pub fn transition(&mut self, event: WatchEvent) -> WatchOutcome {
        // Row 19: terminal is sticky.
        if self.is_terminal() {
            return WatchOutcome::Continue;
        }

        match event {
            // Row 1.
            WatchEvent::ConnectionEstablished => {
                self.connection_status = ConnectionStatus::Connected;
                WatchOutcome::Continue
            }

            // Rows 2, 3, and 13: transport-bucket reconnects (transport
            // error, unexpected EOF, heartbeat starvation) all use
            // exponential backoff per D2.
            WatchEvent::ConnectionLost { .. } | WatchEvent::HeartbeatStarvation => {
                self.connection_status = ConnectionStatus::Reconnecting;
                WatchOutcome::Reconnect {
                    policy: ReconnectPolicy::ExponentialBackoff,
                }
            }

            // Rows 4, 5, 6a-d.
            WatchEvent::ServerClose { reason } => self.handle_server_close(reason),

            // Row 7.
            WatchEvent::BackoffStarted(duration) => {
                self.connection_status = ConnectionStatus::BackoffWait(duration);
                WatchOutcome::Continue
            }

            // Rows 8a and 8b.
            WatchEvent::BackoffElapsed => {
                if matches!(self.connection_status, ConnectionStatus::BackoffWait(_)) {
                    self.connection_status = ConnectionStatus::Reconnecting;
                }
                WatchOutcome::Continue
            }

            // Row 9.
            WatchEvent::AuthRejected => {
                self.connection_status = ConnectionStatus::RefreshingAuth;
                WatchOutcome::RefreshAuth
            }

            // Row 10.
            WatchEvent::AuthRefreshCompleted { success: true } => {
                self.connection_status = ConnectionStatus::Reconnecting;
                WatchOutcome::Continue
            }

            // Row 11.
            WatchEvent::AuthRefreshCompleted { success: false } => {
                self.close_with(CloseReason::Fatal {
                    kind: FatalKind::AuthenticationRejectedAfterRefresh,
                })
            }

            // Rows 12 and 14: pure observation, no state change.
            WatchEvent::HeartbeatReceived | WatchEvent::NotificationReceived { .. } => {
                WatchOutcome::Continue
            }

            // Rows 15a-d.
            WatchEvent::ReplayCompleted => self.handle_replay_completed(),

            // Row 16.
            WatchEvent::GapDetected(reason) => self.enter_gap(reason),

            // Row 17.
            WatchEvent::Fatal(kind) => self.close_with(CloseReason::Fatal { kind }),

            // Row 18.
            WatchEvent::Stop => self.close_with(CloseReason::UserRequested),
        }
    }

    fn handle_server_close(&mut self, reason: ServerCloseReason) -> WatchOutcome {
        match reason {
            // Row 4.
            ServerCloseReason::MaxDurationReached => {
                self.connection_status = ConnectionStatus::Reconnecting;
                WatchOutcome::Reconnect {
                    policy: ReconnectPolicy::Immediate,
                }
            }
            // Row 5.
            ServerCloseReason::ServerShutdown => {
                self.connection_status = ConnectionStatus::Reconnecting;
                WatchOutcome::Reconnect {
                    policy: ReconnectPolicy::ShortBackoff,
                }
            }
            // Rows 6a-d.
            ServerCloseReason::EndOfStream => self.handle_end_of_stream(),
        }
    }

    fn handle_end_of_stream(&mut self) -> WatchOutcome {
        match self.mode {
            // Row 6a: Watch mode always reconnects immediately.
            WatchMode::Watch => {
                self.connection_status = ConnectionStatus::Reconnecting;
                WatchOutcome::Reconnect {
                    policy: ReconnectPolicy::Immediate,
                }
            }
            WatchMode::ReplayOnly => {
                // Row 6b: ReplayOnly + Replaying{rc=true} -> terminal.
                // Rows 6c and 6d (`if let` else branch): Replaying{rc=false}
                // or non-Replaying both reconnect; 6d should not happen in
                // practice but is defined for completeness.
                if let ReplayPhase::Replaying {
                    replay_completed: true,
                    ..
                } = &self.replay_phase
                {
                    self.close_with(CloseReason::EndOfStream)
                } else {
                    self.connection_status = ConnectionStatus::Reconnecting;
                    WatchOutcome::Reconnect {
                        policy: ReconnectPolicy::Immediate,
                    }
                }
            }
        }
    }

    fn handle_replay_completed(&mut self) -> WatchOutcome {
        match (self.mode, &self.replay_phase) {
            // Row 15a: Watch + Replaying -> Live.
            (WatchMode::Watch, ReplayPhase::Replaying { .. }) => {
                self.replay_phase = ReplayPhase::Live;
            }
            // Row 15b: ReplayOnly + Replaying{rc=false} -> Replaying{rc=true}.
            (
                WatchMode::ReplayOnly,
                ReplayPhase::Replaying {
                    replay_completed: false,
                    start,
                },
            ) => {
                let start = start.clone();
                self.replay_phase = ReplayPhase::Replaying {
                    start,
                    replay_completed: true,
                };
            }
            // Rows 15c and 15d: idempotent in all other non-terminal phases.
            _ => {}
        }
        WatchOutcome::Continue
    }

    fn close_with(&mut self, reason: CloseReason) -> WatchOutcome {
        self.replay_phase = ReplayPhase::Closed {
            reason: reason.clone(),
        };
        WatchOutcome::Stop { reason }
    }

    fn enter_gap(&mut self, reason: GapReason) -> WatchOutcome {
        self.replay_phase = ReplayPhase::GapDetected { reason };
        WatchOutcome::Gap { reason }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: panic-on-unexpected is the expected diagnostic"
)]
mod tests {
    use std::time::Duration;

    use super::super::{
        CloseReason, ConnectionLossReason, ConnectionStatus, FatalKind, GapReason, ReconnectPolicy,
        ReplayPhase, ResumeStart, ServerCloseReason, WatchEvent, WatchMode, WatchOutcome,
        WatchState,
    };

    #[test]
    fn constructor_without_start_enters_live() {
        let s = WatchState::new(WatchMode::Watch, None);
        assert_eq!(s.replay_phase(), &ReplayPhase::Live);
        assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
        assert_eq!(s.mode(), WatchMode::Watch);
        assert!(!s.is_terminal());
    }

    #[test]
    fn constructor_with_sequence_enters_replaying() {
        let s = WatchState::new(WatchMode::Watch, Some(ResumeStart::Sequence(42)));
        assert_eq!(
            s.replay_phase(),
            &ReplayPhase::Replaying {
                start: ResumeStart::Sequence(42),
                replay_completed: false,
            }
        );
    }

    #[test]
    fn constructor_with_date_enters_replaying() {
        let s = WatchState::new(
            WatchMode::ReplayOnly,
            Some(ResumeStart::Date("2026-01-01".into())),
        );
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
        let mut s = WatchState::new(WatchMode::Watch, None);
        let out = s.transition(WatchEvent::ConnectionEstablished);
        assert_eq!(out, WatchOutcome::Continue);
        assert_eq!(s.connection_status(), &ConnectionStatus::Connected);
    }

    #[test]
    fn transport_error_reconnects_with_exponential_backoff() {
        let mut s = WatchState::new(WatchMode::Watch, None);
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
        let mut s = WatchState::new(WatchMode::Watch, None);
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
        let mut s = WatchState::new(WatchMode::Watch, None);
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
        let mut s = WatchState::new(WatchMode::Watch, Some(ResumeStart::Sequence(1)));
        let out = s.transition(WatchEvent::ReplayCompleted);
        assert_eq!(out, WatchOutcome::Continue);
        assert_eq!(s.replay_phase(), &ReplayPhase::Live);
    }

    #[test]
    fn replay_completed_in_replay_only_flips_flag_without_leaving_replaying() {
        let mut s = WatchState::new(WatchMode::ReplayOnly, Some(ResumeStart::Sequence(1)));
        let out = s.transition(WatchEvent::ReplayCompleted);
        assert_eq!(out, WatchOutcome::Continue);
        assert_eq!(
            s.replay_phase(),
            &ReplayPhase::Replaying {
                start: ResumeStart::Sequence(1),
                replay_completed: true,
            }
        );
    }

    #[test]
    fn end_of_stream_in_replay_only_after_replay_completed_terminates() {
        let mut s = WatchState::new(WatchMode::ReplayOnly, Some(ResumeStart::Sequence(1)));
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
        let mut s = WatchState::new(WatchMode::Watch, None);
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
        let mut s = WatchState::new(WatchMode::Watch, None);
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
        let mut s = WatchState::new(WatchMode::Watch, None);
        let _ = s.transition(WatchEvent::ConnectionEstablished);
        let out = s.transition(WatchEvent::AuthRejected);
        assert_eq!(out, WatchOutcome::RefreshAuth);
        assert_eq!(s.connection_status(), &ConnectionStatus::RefreshingAuth);
    }

    #[test]
    fn auth_refresh_failure_terminates_with_specific_fatal_kind() {
        let mut s = WatchState::new(WatchMode::Watch, None);
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
        let mut s = WatchState::new(WatchMode::Watch, None);
        let reason = GapReason::ReplayLimitReached {
            oldest_available: 100,
            requested: 1,
        };
        let out = s.transition(WatchEvent::GapDetected(reason));
        assert_eq!(out, WatchOutcome::Gap { reason });
        assert_eq!(s.replay_phase(), &ReplayPhase::GapDetected { reason });
    }

    #[test]
    fn backoff_started_stores_duration_then_elapsed_returns_to_reconnecting() {
        let mut s = WatchState::new(WatchMode::Watch, None);
        let d = Duration::from_millis(500);
        let _ = s.transition(WatchEvent::BackoffStarted(d));
        assert_eq!(s.connection_status(), &ConnectionStatus::BackoffWait(d));
        let _ = s.transition(WatchEvent::BackoffElapsed);
        assert_eq!(s.connection_status(), &ConnectionStatus::Reconnecting);
    }

    #[test]
    fn terminal_state_swallows_all_subsequent_events() {
        let mut s = WatchState::new(WatchMode::Watch, None);
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
}
