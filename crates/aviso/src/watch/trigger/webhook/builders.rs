//! Webhook-specific [`Trigger`] constructor and setters.
//!
//! Lives next to the webhook dispatcher (rather than at
//! [`crate::watch::trigger`] module root) to keep the parent module
//! under the 500-LOC threshold AGENTS.md enforces. The public path
//! `aviso::watch::trigger::DEFAULT_WEBHOOK_TIMEOUT` is preserved
//! verbatim through a re-export in `super::super`.

use super::super::Trigger;
use super::super::kind::TriggerKind;
use super::HttpMethod;

/// Default per-trigger timeout for the webhook trigger when the user
/// has not overridden it via [`Trigger::timeout`]. 30 seconds matches
/// the budget for typical operator-facing receivers (Slack, Teams,
/// Discord all respond well under a second; the cushion absorbs
/// transient backend slowness without prematurely timing out).
pub const DEFAULT_WEBHOOK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

impl Trigger {
    /// Build a webhook trigger that sends an HTTP request per
    /// notification to `url` (template-rendered at dispatch time).
    ///
    /// Default method is [`HttpMethod::Post`]. Default body is the
    /// notification serialised as compact JSON (matching the echo
    /// trigger's output shape). Default `Content-Type` header is
    /// `application/json` when the user has not set one. Default
    /// per-trigger timeout is [`DEFAULT_WEBHOOK_TIMEOUT`].
    ///
    /// URL, header VALUES, and body run through the in-crate
    /// template engine: `{{ notification.<path> }}` substitutes a
    /// notification field; `{{ env.<NAME> }}` substitutes a process
    /// environment variable. Header NAMES are literal.
    ///
    /// # HTTP-status retry semantics
    ///
    /// 5xx responses and transport errors (DNS, TCP, TLS,
    /// mid-stream interrupt) are retryable through the configured
    /// [`Self::retries`] budget. 4xx responses are terminal under
    /// the default `fail_fast = true` and retryable under
    /// `fail_fast = false` (useful when a downstream receiver
    /// occasionally returns transient 4xx).
    ///
    /// # Shared HTTP client
    ///
    /// The webhook reuses the supervisor's shared
    /// [`reqwest::Client`], so any TLS configuration there
    /// inherits automatically.
    ///
    /// # Dispatch-time failure modes
    ///
    /// The constructor is infallible: every check that can reject
    /// the configuration runs at first dispatch and surfaces as a
    /// typed error. Two distinct error classes can arise from URL,
    /// header value, or body input:
    ///
    /// - [`super::super::TriggerError::Template`]: the template ENGINE rejected
    ///   the input. Either the template syntax was malformed
    ///   (unclosed `{{`, empty path segment, unknown namespace:
    ///   `TemplateErrorKind::BadSyntax`) or a substitution failed
    ///   at render time (`{{ notification.<path> }}` did not
    ///   resolve: `Missing`; `{{ env.<NAME> }}` was not set or
    ///   not valid UTF-8: `EnvNotSet` / `EnvNotUnicode`).
    /// - [`super::super::TriggerError::WebhookBuild`]: the HTTP client rejected
    ///   the rendered input. Template syntax was valid and every
    ///   substitution resolved, but the rendered URL is not a
    ///   valid URL (no scheme, embedded whitespace, etc.) or a
    ///   rendered header value contains invalid characters (e.g.
    ///   a newline injected through `{{ env.<NAME> }}`).
    ///
    /// Both variants are terminal under `fail_fast = true` because
    /// the failure is deterministic with respect to the current
    /// notification and process environment.
    #[must_use]
    pub fn webhook(url: impl Into<String>) -> Self {
        Self {
            kind: TriggerKind::Webhook(Box::new(super::build_webhook_config(url))),
            retries: 0,
            required: true,
            timeout: Some(DEFAULT_WEBHOOK_TIMEOUT),
            fail_fast: true,
        }
    }

    /// Override the HTTP method on a webhook trigger. Silently
    /// ignored on echo, log, and command triggers (the method only
    /// applies to webhook).
    #[must_use]
    pub fn method(mut self, method: HttpMethod) -> Self {
        if let TriggerKind::Webhook(cfg) = &mut self.kind {
            super::webhook_set_method(cfg, method);
        }
        self
    }

    /// Add an HTTP header to a webhook trigger. Repeatable; the
    /// same key may be added multiple times to send the header
    /// twice. Header NAMES are taken literally; header VALUES are
    /// template-rendered at dispatch. Silently ignored on echo,
    /// log, and command triggers.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        if let TriggerKind::Webhook(cfg) = &mut self.kind {
            super::webhook_add_header(cfg, name, value);
        }
        self
    }

    /// Override the request body with a template string. Silently
    /// ignored on echo, log, and command triggers. When unset, the
    /// body defaults to the notification serialised as compact JSON
    /// (matching the echo trigger's output shape).
    #[must_use]
    pub fn body_template(mut self, body: impl Into<String>) -> Self {
        if let TriggerKind::Webhook(cfg) = &mut self.kind {
            super::webhook_set_body_template(cfg, body);
        }
        self
    }
}
