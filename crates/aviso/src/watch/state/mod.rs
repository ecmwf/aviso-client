// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! [`WatchState`]: the orthogonal-product reducer for the watch session.
//!
//! The reducer implements the transition rules specified by D2 (the
//! reconnect classifier and state-machine sketch in
//! `plans/decisions.md`). Each match arm carries a row
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
///
/// All fields are private (D15); use [`Self::watch`] / [`Self::replay_only`]
/// to construct and the accessor methods to read. The struct is also
/// `#[non_exhaustive]` so a future public field cannot become a
/// breaking change for downstream callers.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchState {
    replay_phase: ReplayPhase,
    connection_status: ConnectionStatus,
    mode: WatchMode,
}

impl WatchState {
    /// Construct a [`WatchMode::Watch`] session.
    ///
    /// `start = None` enters [`ReplayPhase::Live`] directly (no replay
    /// to do, the supervisor sends neither `from_id` nor `from_date`).
    /// `start = Some(_)` enters [`ReplayPhase::Replaying`] with
    /// `replay_completed: false`. [`ConnectionStatus`] starts as
    /// `Reconnecting` because no transport is open yet.
    #[must_use]
    pub fn watch(start: Option<ResumeStart>) -> Self {
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
            mode: WatchMode::Watch,
        }
    }

    /// Construct a [`WatchMode::ReplayOnly`] session.
    ///
    /// `start` is mandatory because a replay-only session with no
    /// resume position has nothing to replay; the natural-termination
    /// path requires landing in [`ReplayPhase::Replaying`] before
    /// `replay_completed` and `end_of_stream` can fire. The reducer
    /// enters `Replaying { start, replay_completed: false }` with
    /// `ConnectionStatus::Reconnecting`.
    #[must_use]
    pub fn replay_only(start: ResumeStart) -> Self {
        Self {
            replay_phase: ReplayPhase::Replaying {
                start,
                replay_completed: false,
            },
            connection_status: ConnectionStatus::Reconnecting,
            mode: WatchMode::ReplayOnly,
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
        // The match is exhaustive over `(WatchMode, &ReplayPhase)`.
        // Both enums are `#[non_exhaustive]` for downstream callers but
        // exhaustive matching is allowed within the defining crate; a
        // future enum variant therefore breaks compilation here.
        //
        // The two real transitions (Rows 15a, 15b) are spelled out
        // first. Every other combination shares a no-op arm because
        // their behaviour is identical:
        //   - Rows 15c and 15d are idempotent by spec (ReplayOnly +
        //     Replaying{rc=true}, Watch + Live or GapDetected,
        //     ReplayOnly + GapDetected).
        //   - `ReplayPhase::Closed` is filtered by the terminal-sticky
        //     early return at the top of `transition`; unreachable
        //     here in practice.
        //   - `(WatchMode::ReplayOnly, ReplayPhase::Live)` is forbidden
        //     by the `replay_only` constructor; unreachable by
        //     construction.
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
            (
                WatchMode::Watch,
                ReplayPhase::Live | ReplayPhase::GapDetected { .. } | ReplayPhase::Closed { .. },
            )
            | (
                WatchMode::ReplayOnly,
                ReplayPhase::Replaying {
                    replay_completed: true,
                    ..
                }
                | ReplayPhase::Live
                | ReplayPhase::GapDetected { .. }
                | ReplayPhase::Closed { .. },
            ) => {}
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
mod tests;
