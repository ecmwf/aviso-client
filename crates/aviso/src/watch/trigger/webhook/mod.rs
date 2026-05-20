//! Webhook trigger dispatch.
//!
//! POSTs (or GETs/PUTs/PATCHes/DELETEs, per the user's
//! [`super::HttpMethod`] choice) per notification to a user-configured
//! URL. URL, header VALUES (not names), and body all run through the
//! in-crate template engine: `{{ notification.<path> }}` substitutes
//! a notification field; `{{ env.<NAME> }}` reads from the process
//! environment. The default body is the notification serialised as
//! compact JSON (matching the echo trigger's shape); the default
//! `Content-Type` is `application/json` when the user has not set
//! their own header.
//!
//! The HTTP request reuses the supervisor's shared
//! [`reqwest::Client`], so any TLS configuration on the client
//! (root certificates, `danger_accept_invalid_certs`) inherits
//! automatically.
//!
//! # Body capture
//!
//! The response body is captured into a 4 KiB ring buffer via the
//! streaming [`reqwest::Response::chunk`] primitive (paralleling
//! `command/mod.rs::drain_to_ring`). Memory stays bounded at
//! `O(RING_CAP)` regardless of how many bytes the server streams in
//! the request-timeout window; a misbehaving server cannot OOM the
//! client through an oversized body.
//!
//! # Retry classifier
//!
//! 5xx and transport errors are retryable through the per-trigger
//! retry budget. 4xx is terminal under the default
//! `fail_fast = true` and retryable under `fail_fast = false`. The
//! classifier extension lives in
//! [`crate::watch::trigger::dispatcher`].

use std::time::Duration;

use crate::Notification;

use super::TriggerError;
use super::http_method::HttpMethod;
use super::template::{
    CompiledTemplate, TemplateError, TemplateErrorKind, compile, template_error_to_trigger_error,
};

#[cfg(test)]
mod tests;

/// Bytes of response body to retain. The tail is what surfaces in
/// [`TriggerError::Webhook`] on a non-2xx response.
const RING_CAP: usize = 4096;

/// Default `Content-Type` header value injected when the user has not
/// supplied a `Content-Type` header explicitly. Matches the
/// default-body shape (compact JSON of the notification).
const DEFAULT_CONTENT_TYPE: &str = "application/json";

/// Configuration for the webhook trigger, held inside
/// [`super::TriggerKind::Webhook`]. Mirrors the redaction discipline
/// established by [`super::CommandConfig`]: secret-bearing surfaces
/// (URL, header values, body template) never appear in Debug output.
#[derive(Clone)]
pub(super) struct WebhookConfig {
    pub(super) url_template: Result<CompiledTemplate, TemplateError>,
    pub(super) method: HttpMethod,
    pub(super) headers: Vec<(String, Result<CompiledTemplate, TemplateError>)>,
    pub(super) body_template: Option<Result<CompiledTemplate, TemplateError>>,
}

/// Manual Debug impl that REDACTS the URL template, header value
/// templates, and body template, all of which may carry secrets
/// (bearer tokens baked into a URL, Auth headers, signed payload
/// bodies). The redacted output surfaces only structural facts:
/// whether each template compiled, the count of headers, and the
/// chosen method.
impl std::fmt::Debug for WebhookConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let WebhookConfig {
            url_template,
            method,
            headers,
            body_template,
        } = self;
        let url_state: &dyn std::fmt::Display = match url_template {
            Ok(_) => &"<compiled-url-template-redacted>",
            Err(_) => &"<bad-url-template-redacted>",
        };
        let body_state: &dyn std::fmt::Display = match body_template {
            None => &"<default-notification-json>",
            Some(Ok(_)) => &"<compiled-body-template-redacted>",
            Some(Err(_)) => &"<bad-body-template-redacted>",
        };
        f.debug_struct("WebhookConfig")
            .field("url_template", &format_args!("{url_state}"))
            .field("method", method)
            .field("header_count", &headers.len())
            .field("body_template", &format_args!("{body_state}"))
            .finish()
    }
}

/// Crate-private struct holding the rendered URL, headers, and body
/// for one webhook attempt. Surfaced by
/// [`render_webhook_parts_with_env`] so unit tests can assert on
/// templated output without hitting the network.
#[derive(Debug)]
pub(super) struct RenderedWebhook {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// Renders all webhook parts via the process environment for
/// `{{ env.<NAME> }}` substitution. Thin wrapper that delegates to
/// [`render_webhook_parts_with_env`] with `std::env::var`.
fn render_webhook_parts(
    cfg: &WebhookConfig,
    notification: &Notification,
) -> Result<RenderedWebhook, TriggerError> {
    render_webhook_parts_with_env(cfg, notification, |name| match std::env::var(name) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Err(TemplateErrorKind::EnvNotSet),
        Err(std::env::VarError::NotUnicode(_)) => Err(TemplateErrorKind::EnvNotUnicode),
    })
}

/// Renders all webhook parts with an injected env resolver. The
/// resolver returns `Ok(value)` when the variable is set and usable,
/// or `Err(kind)` to surface a typed template error. Production wires
/// `std::env::var` and maps `VarError::NotPresent` to `EnvNotSet`,
/// `VarError::NotUnicode` to `EnvNotUnicode`. Unit tests pass a
/// closure returning hardcoded values; the helper is always
/// compiled, not `#[cfg(test)]`-gated, so production
/// [`render_webhook_parts`] can call into it.
///
/// Header NAMES are taken literally. Header VALUES, the URL, and the
/// body template are rendered through the template engine.
pub(super) fn render_webhook_parts_with_env<F>(
    cfg: &WebhookConfig,
    notification: &Notification,
    env_resolver: F,
) -> Result<RenderedWebhook, TriggerError>
where
    F: Fn(&str) -> Result<String, TemplateErrorKind>,
{
    let url_template = cfg
        .url_template
        .as_ref()
        .map_err(|e| template_error_to_trigger_error(e.clone(), "webhook url"))?;
    let url = url_template
        .render_with_env(notification, &env_resolver)
        .map_err(|e| template_error_to_trigger_error(e, "webhook url"))?;

    let mut headers: Vec<(String, String)> = Vec::with_capacity(cfg.headers.len());
    for (name, value_template) in &cfg.headers {
        let tmpl = value_template
            .as_ref()
            .map_err(|e| template_error_to_trigger_error(e.clone(), "webhook header"))?;
        let value = tmpl
            .render_with_env(notification, &env_resolver)
            .map_err(|e| template_error_to_trigger_error(e, "webhook header"))?;
        headers.push((name.clone(), value));
    }

    let body = match &cfg.body_template {
        Some(template_result) => {
            let tmpl = template_result
                .as_ref()
                .map_err(|e| template_error_to_trigger_error(e.clone(), "webhook body"))?;
            tmpl.render_with_env(notification, &env_resolver)
                .map_err(|e| template_error_to_trigger_error(e, "webhook body"))?
        }
        None => serde_json::to_string(notification).map_err(TriggerError::Encode)?,
    };

    Ok(RenderedWebhook { url, headers, body })
}

/// Dispatch a single webhook trigger attempt.
///
/// Renders the URL, headers, and body; sends an HTTP request via the
/// supervisor's shared `reqwest` client; captures the response body
/// into a bounded ring buffer; classifies the outcome. Returns
/// `Ok(())` on 2xx; [`TriggerError::Webhook`] on 4xx/5xx or transport
/// error; [`TriggerError::Timeout`] when the per-trigger timeout
/// fires before the response completes.
pub(super) async fn dispatch_webhook(
    cfg: &WebhookConfig,
    http: &reqwest::Client,
    timeout: Option<Duration>,
    notification: &Notification,
) -> Result<(), TriggerError> {
    let RenderedWebhook { url, headers, body } = render_webhook_parts(cfg, notification)?;

    let mut request = http.request(cfg.method.to_reqwest(), &url);
    let user_set_content_type = headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if !user_set_content_type {
        request = request.header(reqwest::header::CONTENT_TYPE, DEFAULT_CONTENT_TYPE);
    }
    request = request.body(body);
    if let Some(t) = timeout {
        request = request.timeout(t);
    }

    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            let is_timeout = e.is_timeout();
            let is_builder = e.is_builder();
            let is_connect = e.is_connect();
            // Do NOT pass `error = %e` to the tracing event:
            // reqwest's `Display` for these errors includes the
            // rendered URL ("builder error for url (..)" and the
            // same suffix on transport errors), which may carry
            // secrets baked into the URL via `{{ env.<NAME> }}`
            // template substitution. The category booleans below
            // are sufficient for operator diagnosis.
            tracing::debug!(
                event.name = "client.trigger.webhook.send_failed",
                is_timeout,
                is_builder,
                is_connect,
                "webhook request send failed"
            );
            if is_timeout {
                if let Some(t) = timeout {
                    return Err(TriggerError::Timeout(t));
                }
            }
            if is_builder {
                return Err(TriggerError::WebhookBuild {
                    reason: "request build failed (invalid URL or header value)".to_string(),
                });
            }
            return Err(TriggerError::Webhook {
                status: None,
                body_tail: String::new(),
            });
        }
    };

    let drain = capture_body_tail(response).await;
    match drain {
        BodyDrain::Complete { status, body_tail }
        | BodyDrain::PartialNoTimeout { status, body_tail } => {
            if status.is_success() {
                Ok(())
            } else {
                Err(TriggerError::Webhook {
                    status: Some(status),
                    body_tail,
                })
            }
        }
        BodyDrain::Timeout => {
            if let Some(t) = timeout {
                Err(TriggerError::Timeout(t))
            } else {
                // Body-drain reported a timeout error without a per-trigger
                // timeout configured: classify as a transport error so the
                // dispatcher can retry it through the standard budget. This
                // can happen if reqwest's pool connection deadline expires
                // mid-body without the user setting Trigger::timeout.
                Err(TriggerError::Webhook {
                    status: None,
                    body_tail: String::new(),
                })
            }
        }
    }
}

/// Outcome of [`capture_body_tail`].
///
/// `Complete` means every chunk drained successfully through to EOF.
/// `PartialNoTimeout` means a mid-body transport error truncated the
/// tail but the dispatcher should still classify by the response
/// status. `Timeout` means reqwest's per-request timeout fired during
/// body drain; the caller maps it back to [`TriggerError::Timeout`].
enum BodyDrain {
    Complete {
        status: reqwest::StatusCode,
        body_tail: String,
    },
    PartialNoTimeout {
        status: reqwest::StatusCode,
        body_tail: String,
    },
    Timeout,
}

/// Drain the response body into a bounded 4 KiB ring buffer,
/// retaining only the LAST `RING_CAP` bytes. The `chunk()` loop is
/// the streaming primitive; we never buffer the full body.
///
/// Returns [`BodyDrain::Timeout`] if `response.chunk().await`
/// surfaces an error whose `is_timeout()` is true. The caller maps
/// this to `TriggerError::Timeout(t)` when the user configured a
/// per-trigger timeout. A non-timeout mid-body error truncates the
/// ring and returns [`BodyDrain::PartialNoTimeout`]; the caller
/// still classifies the response by status, because the
/// authoritative success/fail signal is the response status code,
/// not body completeness. A clean EOF returns [`BodyDrain::Complete`].
async fn capture_body_tail(mut response: reqwest::Response) -> BodyDrain {
    let status = response.status();
    let mut ring: Vec<u8> = Vec::with_capacity(RING_CAP);
    loop {
        match response.chunk().await {
            Ok(Some(bytes)) => {
                let slice = bytes.as_ref();
                let n = slice.len();
                if ring.len() + n > RING_CAP {
                    if n >= RING_CAP {
                        ring.clear();
                        ring.extend_from_slice(&slice[n - RING_CAP..]);
                        continue;
                    }
                    let overflow = ring.len() + n - RING_CAP;
                    ring.drain(..overflow);
                }
                ring.extend_from_slice(slice);
            }
            Ok(None) => {
                return BodyDrain::Complete {
                    status,
                    body_tail: String::from_utf8_lossy(&ring).into_owned(),
                };
            }
            Err(e) => {
                let timed_out = e.is_timeout();
                // Do NOT pass `error = %e`: reqwest's Display for
                // body-read errors also includes the rendered URL
                // (the same "for url ({url})" suffix that builder
                // errors carry); the URL can hold secrets.
                tracing::debug!(
                    event.name = "client.trigger.webhook.body_read_error",
                    timed_out,
                    "response body read error during chunked drain"
                );
                if timed_out {
                    return BodyDrain::Timeout;
                }
                return BodyDrain::PartialNoTimeout {
                    status,
                    body_tail: String::from_utf8_lossy(&ring).into_owned(),
                };
            }
        }
    }
}

/// Build a [`WebhookConfig`] from a raw URL string. The URL
/// compile result is stored in the config; render-time failures
/// surface at first dispatch as [`TriggerError::Template`]. Headers
/// and body template are added later via the
/// [`crate::watch::Trigger::header`] and
/// [`crate::watch::Trigger::body_template`] builders.
pub(super) fn build_webhook_config(url: impl Into<String>) -> WebhookConfig {
    let url_str = url.into();
    let compiled = compile(&url_str);
    WebhookConfig {
        url_template: compiled,
        method: HttpMethod::Post,
        headers: Vec::new(),
        body_template: None,
    }
}

/// Add a header to an existing config. Header NAME is literal;
/// header VALUE is compiled as a template and rendered at dispatch.
pub(super) fn webhook_add_header(
    cfg: &mut WebhookConfig,
    name: impl Into<String>,
    value: impl Into<String>,
) {
    let value_str = value.into();
    let compiled = compile(&value_str);
    cfg.headers.push((name.into(), compiled));
}

/// Set the body template on an existing config. Replaces any prior
/// `body_template`. When unset, the default body is the notification
/// serialised as compact JSON (computed at dispatch time).
pub(super) fn webhook_set_body_template(cfg: &mut WebhookConfig, body: impl Into<String>) {
    let body_str = body.into();
    let compiled = compile(&body_str);
    cfg.body_template = Some(compiled);
}

/// Override the HTTP method on an existing config.
pub(super) fn webhook_set_method(cfg: &mut WebhookConfig, method: HttpMethod) {
    cfg.method = method;
}
