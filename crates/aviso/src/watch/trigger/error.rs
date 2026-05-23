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
    /// string `"webhook"`. The rendered URL and header values are
    /// deliberately not surfaced through any error variant or
    /// default tracing event emitted by the dispatcher; only the
    /// response status and a 4 KiB body tail surface to the
    /// operator via [`TriggerError::Webhook`].
    Webhook,
    /// The Teams trigger (HTTP request to a Microsoft Teams Workflows
    /// endpoint, with auto-built Adaptive Card body). Same redaction
    /// discipline as `Webhook`: the URL may carry a SAS token in its
    /// query string; the label displays as the bare string `"teams"`.
    Teams,
    /// The post trigger (HTTP POST with a CloudEvent-shaped body
    /// reconstructed from the notification). Same redaction
    /// discipline as `Webhook`. The label displays as `"post"`.
    Post,
}

impl std::fmt::Display for TriggerKindLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Echo => f.write_str("echo"),
            Self::Log { path } => write!(f, "log({})", path.display()),
            #[cfg(unix)]
            Self::Command => f.write_str("command"),
            Self::Webhook => f.write_str("webhook"),
            Self::Teams => f.write_str("teams"),
            Self::Post => f.write_str("post"),
        }
    }
}

/// Error returned by a single trigger dispatch attempt.
///
/// Carried as the `source` of [`crate::ClientError::TriggerFailed`] when a required
/// trigger fails. The variants cover the failure modes the current dispatcher
/// can produce; the enum is `#[non_exhaustive]` so future trigger kinds (for
/// example an email trigger with SMTP-status awareness) can add variants
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
    ///
    /// Display format uses operator-friendly rendering for both
    /// fields: `status=500` (not `Some(500)`) when the response
    /// arrived; `status=<transport error>` when the HTTP client
    /// reported a connect/TLS/mid-stream failure; `body_tail=<empty>`
    /// when the response body was empty or never received.
    #[error("webhook: status={} body_tail={}", render_webhook_status(*status), render_webhook_body(body_tail))]
    Webhook {
        /// HTTP status code if the response made it back from the
        /// server. `None` on transport errors.
        status: Option<reqwest::StatusCode>,
        /// Last 4 KiB of the response body, lossily UTF-8 decoded.
        /// Empty on transport errors.
        body_tail: String,
    },

    /// A webhook trigger could not be built into a request: the
    /// rendered URL was malformed, a rendered header value
    /// contained invalid characters, or some other input that
    /// passed template rendering was rejected by the HTTP client
    /// builder. The dispatcher classifies this variant as terminal
    /// under `fail_fast = true` because the failure is
    /// deterministic with respect to the current notification: the
    /// next attempt would render the same invalid input and fail
    /// identically.
    ///
    /// `reason` is a static category label (`"request build failed
    /// (invalid URL or header value)"`); it deliberately does NOT
    /// include the rendered URL, header values, or the underlying
    /// HTTP client's `Display` message, because the HTTP client's
    /// error string can include the rendered URL (which may carry
    /// secrets from `{{ env.<NAME> }}` substitutions).
    #[error("webhook build: {reason}")]
    WebhookBuild {
        /// Static category label naming the build-time rejection.
        reason: String,
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

/// Render an `Option<reqwest::StatusCode>` for the `TriggerError::Webhook`
/// `Display` impl. The raw `Debug` form (`Some(500)` / `None`) leaks
/// into operator-facing error messages and reads as a Rust type rather
/// than an HTTP status; this helper produces `500` (set) and
/// `<transport error>` (unset), so the resulting message reads naturally.
fn render_webhook_status(status: Option<reqwest::StatusCode>) -> String {
    match status {
        Some(s) => s.as_u16().to_string(),
        None => "<transport error>".to_string(),
    }
}

/// Render the captured response body tail for the `TriggerError::Webhook`
/// `Display` impl. An empty `body_tail` (transport error or genuinely
/// empty response body) is rendered as `<empty>` so the operator's
/// stderr line is not a dangling `body_tail=` with nothing after the
/// equals sign.
fn render_webhook_body(body_tail: &str) -> String {
    if body_tail.is_empty() {
        "<empty>".to_string()
    } else {
        body_tail.to_string()
    }
}
