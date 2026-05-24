//! Trigger kind internals.

use std::path::PathBuf;

use super::TriggerKindLabel;
#[cfg(unix)]
use super::command::CommandConfig;
use super::post::PostConfig;
use super::teams::TeamsConfig;
use super::webhook::WebhookConfig;

/// Internal description of which built-in trigger a [`super::Trigger`] runs.
///
/// Crate-private; downstream callers configure a `Trigger` through the
/// public [`super::Trigger::echo`], [`super::Trigger::log`],
/// [`super::Trigger::command`], and [`super::Trigger::webhook`]
/// constructors, never by naming this enum.
#[derive(Clone)]
pub(super) enum TriggerKind {
    Echo {
        /// Optional listener-attribution label. When `Some`, prepended
        /// to the TTY leader as `(listener: <label>, trigger: echo)`.
        /// The CLI auto-populates from the listener's YAML `name:`.
        label: Option<String>,
    },
    Log {
        path: PathBuf,
    },
    #[cfg(unix)]
    Command(Box<CommandConfig>),
    Webhook(Box<WebhookConfig>),
    Teams(Box<TeamsConfig>),
    Post(Box<PostConfig>),
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
///
/// The `Command` arm carries a redacted body so the public Debug of a
/// `Trigger` never echoes a raw command template (which may contain
/// bearer tokens, connection URIs, or other secrets). The variant
/// formatter prints only structural facts (whether the template
/// compiled, whether a `working_dir` is set, the count of env vars set
/// by the user); the raw command and env values go to DEBUG-level
/// tracing only.
impl std::fmt::Debug for TriggerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Echo { label } => f.debug_struct("Echo").field("label", label).finish(),
            Self::Log { path } => f.debug_struct("Log").field("path", path).finish(),
            #[cfg(unix)]
            Self::Command(cfg) => f.debug_tuple("Command").field(&**cfg).finish(),
            Self::Webhook(cfg) => f.debug_tuple("Webhook").field(&**cfg).finish(),
            Self::Teams(cfg) => f.debug_tuple("Teams").field(&**cfg).finish(),
            Self::Post(cfg) => f.debug_tuple("Post").field(&**cfg).finish(),
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
        TriggerKind::Echo { .. } => TriggerKindLabel::Echo,
        TriggerKind::Log { path } => TriggerKindLabel::Log { path: path.clone() },
        #[cfg(unix)]
        TriggerKind::Command(_) => TriggerKindLabel::Command,
        TriggerKind::Webhook(_) => TriggerKindLabel::Webhook,
        TriggerKind::Teams(_) => TriggerKindLabel::Teams,
        TriggerKind::Post(_) => TriggerKindLabel::Post,
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
        let echo_dbg = format!("{:?}", TriggerKind::Echo { label: None });
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
