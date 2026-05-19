//! Trigger kind internals.

use std::path::PathBuf;

use super::TriggerKindLabel;
use super::command::CommandConfig;

/// Internal description of which built-in trigger a [`super::Trigger`] runs.
///
/// Crate-private; downstream callers configure a `Trigger` through the
/// public [`super::Trigger::echo`], [`super::Trigger::log`], and
/// [`super::Trigger::command`] constructors, never by naming this enum.
#[derive(Clone)]
pub(super) enum TriggerKind {
    Echo,
    Log {
        path: PathBuf,
    },
    Command(Box<CommandConfig>),
    /// Test-only: fails the first `failures_remaining` attempts, then
    /// resolves per `eventual`. Used by unit tests to drive "fail K times
    /// then succeed/fail" patterns deterministically.
    #[cfg(test)]
    TestFailing {
        failures_remaining: std::sync::Arc<std::sync::atomic::AtomicU32>,
        eventual: TestEventual,
    },
    /// Test-only: fails on the Nth invocation across notifications and
    /// succeeds on all others. Used by unit tests to drive "succeed on
    /// N=1, fail on N=2" patterns that share a single trigger config.
    #[cfg(test)]
    TestFailOnCall {
        calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
        fail_on_call: u32,
    },
}

/// Resolution of a test-only [`TriggerKind::TestFailing`] after its
/// `failures_remaining` counter hits zero.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(super) enum TestEventual {
    /// Subsequent calls succeed.
    Succeed,
    /// Subsequent calls also fail (the trigger never recovers).
    Fail,
}

/// Manual `Debug` impl for the same reason as [`super::Trigger`]'s manual impl.
impl std::fmt::Debug for TriggerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Echo => f.debug_struct("Echo").finish(),
            Self::Log { path } => f.debug_struct("Log").field("path", path).finish(),
            Self::Command(cfg) => f.debug_tuple("Command").field(&**cfg).finish(),
            #[cfg(test)]
            Self::TestFailing {
                failures_remaining,
                eventual,
            } => f
                .debug_struct("TestFailing")
                .field("failures_remaining", failures_remaining)
                .field("eventual", eventual)
                .finish(),
            #[cfg(test)]
            Self::TestFailOnCall {
                calls,
                fail_on_call,
            } => f
                .debug_struct("TestFailOnCall")
                .field("calls", calls)
                .field("fail_on_call", fail_on_call)
                .finish(),
        }
    }
}

/// Map an internal kind to its public-facing diagnostic label.
pub(super) fn trigger_kind_label(kind: &TriggerKind) -> TriggerKindLabel {
    match kind {
        TriggerKind::Echo => TriggerKindLabel::Echo,
        TriggerKind::Log { path } => TriggerKindLabel::Log { path: path.clone() },
        TriggerKind::Command(_) => TriggerKindLabel::Command,
        #[cfg(test)]
        TriggerKind::TestFailing { .. } => TriggerKindLabel::Echo,
        #[cfg(test)]
        TriggerKind::TestFailOnCall { .. } => TriggerKindLabel::Echo,
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    reason = "test code: panic on unexpected variant is the standard test diagnostic"
)]
mod tests {
    use std::path::PathBuf;

    use super::TriggerKind;

    #[test]
    fn trigger_kind_debug_includes_log_path() {
        let echo_dbg = format!("{:?}", TriggerKind::Echo);
        assert!(echo_dbg.contains("Echo"));

        let log_dbg = format!(
            "{:?}",
            TriggerKind::Log {
                path: PathBuf::from("/tmp/x.log")
            }
        );
        assert!(log_dbg.contains("Log"), "got: {log_dbg}");
        assert!(log_dbg.contains("/tmp/x.log"), "got: {log_dbg}");
    }
}
