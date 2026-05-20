//! Trigger error and label types.
//!
//! Split out of [`super`]`::mod` to keep that file under the 500-LOC
//! cap (per AGENTS.md "One module per concern. Files >500 lines get
//! split."). The trigger module re-exports both types so downstream
//! callers continue to import them as `crate::watch::TriggerError`
//! and `crate::watch::TriggerKindLabel` without noticing the split.

use std::path::PathBuf;

use super::TemplateErrorKind;

/// Human-readable label naming the kind of trigger that produced a
/// [`crate::ClientError::TriggerFailed`].
///
/// Separate from the crate-private `TriggerKind` enum so future internal
/// variants (or test-only ones) cannot leak into public error displays.
///
/// The `Command` variant intentionally carries no body: a command
/// trigger's full command string may contain secrets (bearer tokens,
/// connection URIs), and any redacted summary is still an attack
/// surface if the secret appears in the visible prefix. The label
/// displays as the bare string `"command"`; the full command goes
/// into DEBUG-level structured tracing only.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerKindLabel {
    /// The echo trigger (NDJSON to standard output).
    Echo,
    /// The log trigger (NDJSON appended to a file at the carried path).
    Log {
        /// The configured log file path.
        path: PathBuf,
    },
    /// The command trigger (subprocess spawn). Carries no body to
    /// avoid leaking secret-bearing command fragments through error
    /// chains; the full command appears in DEBUG-level tracing only.
    /// Unix-only (`#[cfg(unix)]`).
    #[cfg(unix)]
    Command,
    /// The webhook trigger (HTTP request). Carries no body for the
    /// same secret-leak reason as `Command`: webhook URLs and header
    /// values can carry tokens (Slack `?token=...`, GitHub `?key=...`,
    /// Auth headers), and any redacted summary risks leaking the
    /// visible prefix of a secret. The label displays as the bare
    /// string `"webhook"`; the full URL and headers stay in
    /// DEBUG-level structured tracing only.
    Webhook,
}

impl std::fmt::Display for TriggerKindLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Echo => f.write_str("echo"),
            Self::Log { path } => write!(f, "log({})", path.display()),
            #[cfg(unix)]
            Self::Command => f.write_str("command"),
            Self::Webhook => f.write_str("webhook"),
        }
    }
}

/// Error returned by a single trigger dispatch attempt.
///
/// Carried as the `source` of [`crate::ClientError::TriggerFailed`] when a required
/// trigger fails. The variants cover the failure modes the v1 dispatcher
/// can produce; the enum is `#[non_exhaustive]` so future triggers (for
/// example a Webhook trigger with HTTP-status awareness) can add variants
/// without breaking downstream matches.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TriggerError {
    /// Underlying I/O failure (broken pipe writing to stdout, permission
    /// denied opening the log file, disk full, file vanished, and so on).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// `serde_json` refused to serialise the notification. Highly unlikely
    /// for the well-typed `Notification` shape, but the variant exists for
    /// safety.
    #[error("encode notification: {0}")]
    Encode(#[from] serde_json::Error),

    /// A command trigger's child process exited with a non-zero
    /// status. `stderr_tail` is the last 4 KiB of the child's
    /// stderr, captured into a ring buffer; the head is dropped on
    /// overflow. Stdout content is suppressed per the
    /// no-payload-logging rule. Unix-only (`#[cfg(unix)]`).
    #[cfg(unix)]
    #[error("command exited {exit_code}: {stderr_tail}")]
    Command {
        /// Child process's exit code. `-1` when the child died from
        /// a signal (Unix sets the exit code to None for
        /// signal-terminated children; `-1` is the canonical sentinel).
        exit_code: i32,
        /// Last 4 KiB of the child's stderr, lossily UTF-8 decoded.
        stderr_tail: String,
    },

    /// A trigger attempt exceeded its configured per-trigger timeout.
    ///
    /// Surfaced on triggers that have a meaningful timeout
    /// (currently the command trigger and the webhook trigger; echo
    /// and log silently ignore the [`super::Trigger::timeout`]
    /// setter). The carried duration is the timeout that was set,
    /// not the actual elapsed time.
    #[error("trigger timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// A webhook trigger received a non-2xx HTTP response or failed
    /// at the transport layer (DNS, TCP, TLS, mid-stream interrupt).
    /// `status` is `None` on transport failures and `Some(code)` on
    /// every received response that the dispatcher then classified
    /// as a failure. `body_tail` is the last 4 KiB of the response
    /// body, lossily UTF-8 decoded; empty on transport failures.
    #[error("webhook: status={status:?} body_tail={body_tail}")]
    Webhook {
        /// HTTP status code if the response made it back from the
        /// server. `None` on transport errors.
        status: Option<reqwest::StatusCode>,
        /// Last 4 KiB of the response body, lossily UTF-8 decoded.
        /// Empty on transport errors.
        body_tail: String,
    },

    /// A template substitution failed.
    ///
    /// The `context` is a safe static label naming WHICH template
    /// surface produced the error (`"command"`, `"webhook url"`,
    /// `"webhook body"`, `"webhook header"`). It is NOT a snippet of
    /// the raw template source: raw templates may carry secrets
    /// (e.g., a bearer token baked into a webhook URL), so only safe
    /// labels reach the public error chain.
    ///
    /// The `field` names the specific path or env-var name that
    /// failed: a JSON path like `"notification.payload.target"` for
    /// `Missing`, an env-var name like `"SLACK_TOKEN"` for `EnvNotSet`,
    /// or a safe static label like `"unclosed_braces"` for `BadSyntax`.
    /// For `BadSyntax` specifically, the `field` carries no fragment
    /// from the raw template; it names only the parse failure
    /// category.
    ///
    /// The raw template source is logged at `DEBUG` via the
    /// `client.trigger.template.render_failed` tracing event for
    /// operators who control the logging sink, but never appears in
    /// this public variant.
    #[error("template render in {context} failed at {field}: {kind:?}")]
    Template {
        /// Safe static label naming which template surface failed.
        context: String,
        /// Specific path or env-var name that failed; for
        /// `BadSyntax`, a safe static label naming the parse failure.
        field: String,
        /// Categorisation of the failure.
        kind: TemplateErrorKind,
    },
}
