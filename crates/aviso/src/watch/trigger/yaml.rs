//! Declarative YAML configuration for triggers.
//!
//! [`TriggerConfig`] is the serde-derived target the CLI and the
//! future Python bindings use to deserialise listener triggers from a
//! YAML config file. Each variant carries its body as a named payload
//! struct (`EchoConfig`, `LogConfig`, `CommandTriggerConfig`,
//! `WebhookTriggerConfig`) rather than inline-struct variants, so
//! `#[serde(deny_unknown_fields)]` on each payload struct catches
//! misspelled keys reliably (serde-rs issue #2123 makes the same
//! attribute on the outer tagged enum unreliable when the tag matches
//! a known variant). Unknown `type:` values fail automatically via
//! serde's "unknown variant" error without any extra attribute.
//!
//! # Example
//!
//! ```yaml
//! triggers:
//!   - type: webhook
//!     url: "https://hooks.example.org/notify"
//!     headers:
//!       Authorization: "Bearer {{ env.HOOK_TOKEN }}"
//!     body_template: '{"seq": {{ notification.sequence }}}'
//!     timeout: 30s
//!     retries: 3
//!   - type: log
//!     path: "/var/log/aviso/notifications.log"
//!     retries: 2
//! ```
//!
//! Each `TriggerConfig` deserialises into a [`Trigger`] builder via
//! [`TriggerConfig::into_trigger`].

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use super::{HttpMethod, Trigger};

/// Declarative trigger configuration deserialised from YAML.
///
/// Tagged on the `type:` field. Each variant carries a named payload
/// struct so `#[serde(deny_unknown_fields)]` on the struct catches
/// misspelled keys within a matched variant; the outer-enum form
/// does not (serde-rs issue #2123). Unknown `type:` values fail
/// automatically as serde "unknown variant" errors. New variants
/// land additively under `#[non_exhaustive]`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TriggerConfig {
    /// Echo trigger: writes the notification as compact JSON to
    /// standard output.
    Echo(EchoConfig),
    /// Log trigger: appends each notification as a line of compact
    /// JSON to a user-specified file.
    Log(LogConfig),
    /// Command trigger: spawns a `/bin/sh -c <rendered>` child per
    /// notification, exposing notification fields as `AVISO_*`
    /// environment variables. Unix-only (`#[cfg(unix)]`).
    #[cfg(unix)]
    Command(CommandTriggerConfig),
    /// Webhook trigger: sends an HTTP request per notification to a
    /// user-configured URL.
    Webhook(WebhookTriggerConfig),
    /// Teams trigger: sugar over [`Self::Webhook`] that auto-builds an
    /// Adaptive Card body for Microsoft Teams Workflows endpoints.
    /// Operators wanting richer card customisation use [`Self::Webhook`]
    /// directly with a hand-written `body_template`.
    Teams(TeamsTriggerConfig),
    /// Post trigger: HTTP POST with a CloudEvent-shaped body
    /// reconstructed from the notification. Custom headers supported;
    /// body shape is fixed (no `body_template` field). For arbitrary
    /// body shapes, use [`Self::Webhook`] directly.
    Post(PostTriggerConfig),
}

/// YAML payload for [`TriggerConfig::Echo`].
#[non_exhaustive]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EchoConfig {
    /// Retry count; `N` allows up to `N` additional attempts after
    /// the first failure. Default `0`.
    #[serde(default)]
    pub retries: u32,
    /// Required flag. When `true` (default), a final failure
    /// terminates the watch with
    /// [`crate::ClientError::TriggerFailed`]; when `false`, the
    /// failure is logged at `WARN` and the watch continues.
    #[serde(default = "default_required")]
    pub required: bool,
}

/// YAML payload for [`TriggerConfig::Log`].
#[non_exhaustive]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogConfig {
    /// Path to the log file. Opened with `append(true).create(true)`
    /// at first dispatch and held for the trigger's lifetime.
    pub path: PathBuf,
    /// Retry count; `N` allows up to `N` additional attempts after
    /// the first failure. Default `0`.
    #[serde(default)]
    pub retries: u32,
    /// Required flag. When `true` (default), a final failure
    /// terminates the watch; when `false`, the failure is logged at
    /// `WARN` and the watch continues.
    #[serde(default = "default_required")]
    pub required: bool,
}

/// YAML payload for [`TriggerConfig::Command`]. Unix-only.
#[cfg(unix)]
#[non_exhaustive]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandTriggerConfig {
    /// Command template; rendered through the in-crate template
    /// engine and passed to `/bin/sh -c`.
    pub command: String,
    /// Extra environment variables to set on the child process.
    /// User-supplied entries override the dispatcher-injected
    /// `AVISO_*` keys when both are present.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Working directory for the child process. When absent the
    /// child inherits the current directory.
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    /// Per-trigger timeout. Parsed as a humantime duration string
    /// (`30s`, `2m`, `1h30m`, `500ms`). Absent means no timeout.
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
    /// Retry count; default `0`.
    #[serde(default)]
    pub retries: u32,
    /// Required flag; default `true`.
    #[serde(default = "default_required")]
    pub required: bool,
    /// Fail-fast policy on terminal failures; default `true`. For
    /// commands, terminal failures under `fail_fast = true` are
    /// `TriggerError::Command` (non-zero exit) and
    /// `TriggerError::Template` (template engine rejection). I/O
    /// errors and timeouts stay retryable.
    #[serde(default = "default_fail_fast")]
    pub fail_fast: bool,
}

/// YAML payload for [`TriggerConfig::Webhook`].
#[non_exhaustive]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookTriggerConfig {
    /// URL template; rendered through the in-crate template engine
    /// at dispatch time.
    pub url: String,
    /// HTTP method. Defaults to `POST` when absent.
    #[serde(default)]
    pub method: Option<HttpMethod>,
    /// Headers to attach to the request. Header names are taken
    /// literally; header values are template-rendered at dispatch.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Optional body template. When absent, the body defaults to
    /// the notification serialised as compact JSON.
    #[serde(default)]
    pub body_template: Option<String>,
    /// Per-trigger timeout. Parsed as a humantime duration string
    /// (`30s`, `2m`, `1h30m`, `500ms`). When absent the webhook
    /// uses [`crate::watch::DEFAULT_WEBHOOK_TIMEOUT`].
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
    /// Retry count; default `0`.
    #[serde(default)]
    pub retries: u32,
    /// Required flag; default `true`.
    #[serde(default = "default_required")]
    pub required: bool,
    /// Fail-fast policy on terminal failures; default `true`. For
    /// webhooks, terminal failures under `fail_fast = true` are:
    /// `TriggerError::Webhook` with a 4xx HTTP status (the receiver
    /// is rejecting the request); `TriggerError::WebhookBuild` (the
    /// HTTP client rejected the rendered request: malformed URL or
    /// invalid header value); and `TriggerError::Template` (the
    /// template engine rejected the input). 5xx, transport errors,
    /// I/O errors, and timeouts stay retryable.
    #[serde(default = "default_fail_fast")]
    pub fail_fast: bool,
}

/// YAML payload for [`TriggerConfig::Teams`]. Sugar over the webhook
/// trigger targeting Microsoft Teams Workflows endpoints; desugars to
/// a [`TriggerConfig::Webhook`] at `into_trigger` time.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamsTriggerConfig {
    /// Workflow URL. Template-rendered at dispatch time;
    /// `{{ env.TEAMS_WEBHOOK_URL }}` is the canonical secret-bearing pattern.
    pub url: String,
    /// Optional title template for the Adaptive Card's first TextBlock.
    /// Defaults to `aviso {{ notification.event_type }} #{{ notification.sequence }}`.
    #[serde(default = "default_teams_title")]
    pub title_template: String,
    /// Per-trigger timeout. Parsed as a humantime duration string.
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
    /// Retry count; default `0`.
    #[serde(default)]
    pub retries: u32,
    /// Required flag; default `true`.
    #[serde(default = "default_required")]
    pub required: bool,
    /// Fail-fast policy; default `true`. Inherits the webhook classifier.
    #[serde(default = "default_fail_fast")]
    pub fail_fast: bool,
}

fn default_teams_title() -> String {
    super::teams::DEFAULT_TEAMS_TITLE_TEMPLATE.to_string()
}

/// YAML payload for [`TriggerConfig::Post`]. Migration-friendly shape
/// for operators coming from pyaviso's `post` trigger: URL plus
/// optional custom headers; the body is a fixed CloudEvent envelope
/// built from the notification at dispatch time (no `body_template`
/// field).
#[non_exhaustive]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostTriggerConfig {
    /// URL template; rendered through the in-crate template engine at
    /// dispatch time.
    pub url: String,
    /// Headers to attach to the request. Header names are taken
    /// literally; header values are template-rendered at dispatch.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Per-trigger timeout. Parsed as a humantime duration string.
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
    /// Retry count; default `0`.
    #[serde(default)]
    pub retries: u32,
    /// Required flag; default `true`.
    #[serde(default = "default_required")]
    pub required: bool,
    /// Fail-fast policy; default `true`. Inherits the webhook classifier.
    #[serde(default = "default_fail_fast")]
    pub fail_fast: bool,
}

impl TriggerConfig {
    /// Convert the declarative config into a strongly-typed
    /// [`Trigger`] builder, applying all per-variant defaults.
    #[must_use]
    pub fn into_trigger(self) -> Trigger {
        match self {
            Self::Echo(cfg) => Trigger::echo().retries(cfg.retries).required(cfg.required),
            Self::Log(cfg) => Trigger::log(cfg.path)
                .retries(cfg.retries)
                .required(cfg.required),
            #[cfg(unix)]
            Self::Command(cfg) => {
                let mut t = Trigger::command(cfg.command);
                for (k, v) in cfg.env {
                    t = t.env(k, v);
                }
                if let Some(d) = cfg.working_dir {
                    t = t.working_dir(d);
                }
                if let Some(d) = cfg.timeout {
                    t = t.timeout(d);
                }
                t.retries(cfg.retries)
                    .required(cfg.required)
                    .fail_fast(cfg.fail_fast)
            }
            Self::Webhook(cfg) => {
                let mut t = Trigger::webhook(cfg.url);
                if let Some(m) = cfg.method {
                    t = t.method(m);
                }
                for (k, v) in cfg.headers {
                    t = t.header(k, v);
                }
                if let Some(b) = cfg.body_template {
                    t = t.body_template(b);
                }
                if let Some(d) = cfg.timeout {
                    t = t.timeout(d);
                }
                t.retries(cfg.retries)
                    .required(cfg.required)
                    .fail_fast(cfg.fail_fast)
            }
            Self::Teams(cfg) => {
                let mut t = Trigger::teams(cfg.url).title_template(cfg.title_template);
                if let Some(d) = cfg.timeout {
                    t = t.timeout(d);
                }
                t.retries(cfg.retries)
                    .required(cfg.required)
                    .fail_fast(cfg.fail_fast)
            }
            Self::Post(cfg) => {
                let mut t = Trigger::post(cfg.url);
                for (k, v) in cfg.headers {
                    t = t.post_header(k, v);
                }
                if let Some(d) = cfg.timeout {
                    t = t.timeout(d);
                }
                t.retries(cfg.retries)
                    .required(cfg.required)
                    .fail_fast(cfg.fail_fast)
            }
        }
    }
}

fn default_required() -> bool {
    true
}

fn default_fail_fast() -> bool {
    true
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap on YAML deserialisation success and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use super::{HttpMethod, TriggerConfig};
    use crate::watch::trigger::kind::TriggerKind;

    fn parse(yaml: &str) -> TriggerConfig {
        serde_norway::from_str::<TriggerConfig>(yaml).unwrap()
    }

    fn parse_err(yaml: &str) -> serde_norway::Error {
        serde_norway::from_str::<TriggerConfig>(yaml).unwrap_err()
    }

    #[test]
    fn yaml_deserialise_echo_with_defaults() {
        let cfg = parse("type: echo\n");
        match cfg {
            TriggerConfig::Echo(echo) => {
                assert_eq!(echo.retries, 0);
                assert!(echo.required);
            }
            other => panic!("expected Echo, got {other:?}"),
        }
    }

    #[test]
    fn yaml_deserialise_echo_with_overrides() {
        let cfg = parse("type: echo\nretries: 5\nrequired: false\n");
        match cfg {
            TriggerConfig::Echo(echo) => {
                assert_eq!(echo.retries, 5);
                assert!(!echo.required);
            }
            other => panic!("expected Echo, got {other:?}"),
        }
    }

    #[test]
    fn yaml_deserialise_log_with_path() {
        let cfg = parse("type: log\npath: /tmp/foo.log\n");
        match cfg {
            TriggerConfig::Log(log) => {
                assert_eq!(log.path.to_str(), Some("/tmp/foo.log"));
                assert_eq!(log.retries, 0);
                assert!(log.required);
            }
            other => panic!("expected Log, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn yaml_deserialise_command_with_full_fields() {
        let yaml = r#"
type: command
command: "echo hi"
env:
  KEY1: value1
  KEY2: value2
working_dir: /tmp
timeout: 30s
retries: 2
required: false
fail_fast: false
"#;
        let cfg = parse(yaml);
        match cfg {
            TriggerConfig::Command(cmd) => {
                assert_eq!(cmd.command, "echo hi");
                assert_eq!(cmd.env.len(), 2);
                assert_eq!(cmd.env.get("KEY1").map(String::as_str), Some("value1"));
                assert_eq!(
                    cmd.working_dir.as_deref().and_then(std::path::Path::to_str),
                    Some("/tmp")
                );
                assert_eq!(cmd.timeout, Some(std::time::Duration::from_secs(30)));
                assert_eq!(cmd.retries, 2);
                assert!(!cmd.required);
                assert!(!cmd.fail_fast);
            }
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn yaml_deserialise_webhook_with_full_fields() {
        let yaml = r#"
type: webhook
url: "https://hooks.example.org/notify"
method: PUT
headers:
  X-Foo: bar
  Authorization: "Bearer secret"
body_template: '{"k":"v"}'
timeout: 1m30s
retries: 3
required: false
fail_fast: false
"#;
        let cfg = parse(yaml);
        match cfg {
            TriggerConfig::Webhook(wh) => {
                assert_eq!(wh.url, "https://hooks.example.org/notify");
                assert_eq!(wh.method, Some(HttpMethod::Put));
                assert_eq!(wh.headers.len(), 2);
                assert_eq!(wh.body_template.as_deref(), Some("{\"k\":\"v\"}"));
                assert_eq!(wh.timeout, Some(std::time::Duration::from_secs(90)));
                assert_eq!(wh.retries, 3);
                assert!(!wh.required);
                assert!(!wh.fail_fast);
            }
            other => panic!("expected Webhook, got {other:?}"),
        }
    }

    #[test]
    fn yaml_deserialise_timeout_humantime_formats() {
        for (input, expected_secs) in [("30s", 30), ("2m", 120), ("1h", 3600), ("1h30m", 5400)] {
            let yaml = format!("type: webhook\nurl: https://x\ntimeout: {input}\n");
            let cfg = parse(&yaml);
            match cfg {
                TriggerConfig::Webhook(wh) => assert_eq!(
                    wh.timeout,
                    Some(std::time::Duration::from_secs(expected_secs)),
                    "input: {input}"
                ),
                other => panic!("expected Webhook, got {other:?}"),
            }
        }
    }

    #[test]
    fn yaml_deserialise_timeout_milliseconds_format() {
        let cfg = parse("type: webhook\nurl: https://x\ntimeout: 500ms\n");
        match cfg {
            TriggerConfig::Webhook(wh) => {
                assert_eq!(wh.timeout, Some(std::time::Duration::from_millis(500)));
            }
            other => panic!("expected Webhook, got {other:?}"),
        }
    }

    #[test]
    fn yaml_deserialise_unknown_type_fails_with_clear_message() {
        let err = parse_err("type: comand\n");
        let message = err.to_string();
        assert!(
            message.contains("comand") || message.contains("unknown variant"),
            "error should name the bad variant: {message}"
        );
    }

    #[test]
    fn yaml_deserialise_unknown_field_within_echo_fails() {
        let err = parse_err("type: echo\nbogus_field: 1\n");
        let message = err.to_string();
        assert!(
            message.contains("bogus_field") || message.contains("unknown field"),
            "error should name the bad field: {message}"
        );
    }

    #[test]
    fn yaml_deserialise_unknown_field_within_log_fails() {
        let err = parse_err("type: log\npath: /tmp/x\nbogus_field: 1\n");
        let message = err.to_string();
        assert!(
            message.contains("bogus_field") || message.contains("unknown field"),
            "error should name the bad field: {message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn yaml_deserialise_unknown_field_within_command_fails() {
        let err = parse_err("type: command\ncommand: \"true\"\nbogus_field: 1\n");
        let message = err.to_string();
        assert!(
            message.contains("bogus_field") || message.contains("unknown field"),
            "error should name the bad field: {message}"
        );
    }

    #[test]
    fn yaml_deserialise_unknown_field_within_webhook_fails() {
        let err = parse_err("type: webhook\nurl: https://x\nbogus_field: 1\n");
        let message = err.to_string();
        assert!(
            message.contains("bogus_field") || message.contains("unknown field"),
            "error should name the bad field: {message}"
        );
    }

    #[test]
    fn yaml_method_uppercase_required() {
        let cfg = parse("type: webhook\nurl: https://x\nmethod: POST\n");
        match cfg {
            TriggerConfig::Webhook(wh) => assert_eq!(wh.method, Some(HttpMethod::Post)),
            other => panic!("expected Webhook, got {other:?}"),
        }
    }

    #[test]
    fn yaml_method_lowercase_fails() {
        let err = parse_err("type: webhook\nurl: https://x\nmethod: post\n");
        let message = err.to_string();
        assert!(
            message.contains("unknown variant") || message.contains("post"),
            "lowercase method should fail: {message}"
        );
    }

    #[test]
    fn yaml_to_trigger_round_trip_echo() {
        let cfg = parse("type: echo\nretries: 3\nrequired: false\n");
        let trigger = cfg.into_trigger();
        assert_eq!(trigger.retries, 3);
        assert!(!trigger.required);
        assert!(matches!(trigger.kind, TriggerKind::Echo { .. }));
    }

    #[test]
    fn yaml_to_trigger_round_trip_webhook_applies_method_header_body() {
        let yaml = r#"
type: webhook
url: "https://hooks.example.org/notify"
method: PATCH
headers:
  X-Custom: value
body_template: '{"x":1}'
timeout: 5s
retries: 1
"#;
        let cfg = parse(yaml);
        let trigger = cfg.into_trigger();
        assert_eq!(trigger.retries, 1);
        assert!(trigger.required);
        assert_eq!(trigger.timeout, Some(std::time::Duration::from_secs(5)));
        assert!(matches!(trigger.kind, TriggerKind::Webhook(_)));
    }

    #[test]
    fn yaml_deserialise_teams_with_defaults() {
        let cfg = parse("type: teams\nurl: \"https://example/workflow\"\n");
        match cfg {
            TriggerConfig::Teams(teams) => {
                assert_eq!(teams.url, "https://example/workflow");
                assert!(
                    teams.title_template.contains("notification.event_type"),
                    "default title template must reference notification.event_type: {}",
                    teams.title_template
                );
                assert_eq!(teams.retries, 0);
                assert!(teams.required);
            }
            other => panic!("expected Teams, got {other:?}"),
        }
    }

    #[test]
    fn yaml_deserialise_teams_with_explicit_title_template() {
        let yaml = r#"
type: teams
url: "{{ env.TEAMS_WEBHOOK_URL }}"
title_template: "custom: {{ notification.event_type }} #{{ notification.sequence }}"
retries: 2
timeout: 10s
"#;
        let cfg = parse(yaml);
        match cfg {
            TriggerConfig::Teams(teams) => {
                assert_eq!(teams.url, "{{ env.TEAMS_WEBHOOK_URL }}");
                assert_eq!(
                    teams.title_template,
                    "custom: {{ notification.event_type }} #{{ notification.sequence }}"
                );
                assert_eq!(teams.retries, 2);
                assert_eq!(teams.timeout, Some(std::time::Duration::from_secs(10)));
            }
            other => panic!("expected Teams, got {other:?}"),
        }
    }

    #[test]
    fn yaml_teams_to_trigger_produces_teams_kind() {
        let yaml = r#"
type: teams
url: "https://example/workflow"
title_template: "aviso fires"
"#;
        let cfg = parse(yaml);
        let trigger = cfg.into_trigger();
        assert!(
            matches!(trigger.kind, TriggerKind::Teams(_)),
            "type: teams must produce TriggerKind::Teams (was a webhook desugaring before; now a proper kind that builds the Adaptive Card from the notification at dispatch time)"
        );
        assert_eq!(trigger.retries, 0);
        assert!(trigger.required);
    }
}
