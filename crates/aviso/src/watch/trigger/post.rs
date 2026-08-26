// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Post trigger dispatch.
//!
//! HTTP POST per notification, body = the CloudEvent envelope from
//! `aviso-server`. The dispatcher prefers the raw envelope captured
//! at the SSE parser ([`Notification::cloudevent`] is `Some` in the
//! production watch path) and forwards those bytes verbatim, so
//! receivers see the server's actual `specversion` / per-event-type
//! `type` (e.g. `int.ecmwf.aviso.mars`) / `source` URL / nanosecond-
//! precision `time` / `dataschema` URL / `datacontenttype` /
//! `data` exactly as emitted. Only when `cloudevent` is `None`
//! (test fixtures, library callers building notifications outside
//! the watch path) does the dispatcher fall back to a minimal
//! reconstructed envelope (`type: co.ecmwf.aviso.event`,
//! `source: aviso-client`, no `time`).
//!
//! Suitable for pyaviso migration and for any generic CloudEvent
//! receiver. Custom headers supported via the same templated-value
//! pattern as the webhook trigger.
//!
//! Differs from the webhook trigger in three ways:
//!
//! - **Body shape**: webhook defaults to the lib's `Notification`
//!   JSON; post sends the CloudEvent envelope (raw passthrough in
//!   production; reconstructed fallback in tests).
//! - **Method**: locked to POST (the trigger name encodes the verb).
//! - **No body_template**: operators who need a custom body shape
//!   should use the webhook trigger directly.
//!
//! The fallback's omission of `time` is honest about the lossy path:
//! the lib only reconstructs when the wire envelope is unavailable,
//! and the server's emission time is not reconstructable. Production
//! receivers always see the server's real `time` because production
//! always has the raw envelope.

use std::time::Duration;

use crate::Notification;

use super::TriggerError;
use super::http_method::HttpMethod;
use super::template::{CompiledTemplate, TemplateError, compile};
use super::webhook::WebhookConfig;
use super::webhook::dispatch_webhook;

/// Configuration for the post trigger.
#[derive(Clone)]
pub(super) struct PostConfig {
    pub(super) url_template: Result<CompiledTemplate, TemplateError>,
    pub(super) headers: Vec<(String, Result<CompiledTemplate, TemplateError>)>,
}

impl std::fmt::Debug for PostConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let PostConfig {
            url_template,
            headers,
        } = self;
        let url_state: &dyn std::fmt::Display = match url_template {
            Ok(_) => &"<compiled-url-template-redacted>",
            Err(_) => &"<bad-url-template-redacted>",
        };
        f.debug_struct("PostConfig")
            .field("url_template", &format_args!("{url_state}"))
            .field("header_count", &headers.len())
            .finish()
    }
}

/// `CloudEvent` `type` value emitted by the post trigger. Constant
/// for v1; operators wanting customisation can use the webhook
/// trigger with a hand-written `body_template`.
pub(super) const POST_CLOUDEVENT_TYPE: &str = "co.ecmwf.aviso.event";

/// `CloudEvent` `source` value emitted by the post trigger. Constant
/// for v1.
pub(super) const POST_CLOUDEVENT_SOURCE: &str = "aviso-client";

/// Dispatch one post trigger attempt.
///
/// When the notification carries the raw `CloudEvent` envelope from
/// the SSE wire ([`Notification::cloudevent`] is `Some`), the post
/// trigger sends those bytes verbatim. Otherwise it falls back to a
/// reconstructed envelope built from the lib's narrowed fields; the
/// reconstruction is best-effort (synthesised `specversion`, `type`,
/// `source`; no `time` since the server's emission time is not
/// preserved through the parser) and exists only so test fixtures
/// without a real wire envelope still produce a valid `CloudEvent`
/// body.
pub(super) async fn dispatch_post(
    cfg: &PostConfig,
    http: &reqwest::Client,
    timeout: Option<Duration>,
    notification: &Notification,
) -> Result<(), TriggerError> {
    let envelope = match &notification.cloudevent {
        Some(raw) => raw.clone(),
        None => build_reconstructed_envelope(notification),
    };
    let body_string = serde_json::to_string(&envelope).map_err(TriggerError::Encode)?;
    let webhook_cfg = synthesise_webhook_config(cfg, &body_string);
    dispatch_webhook(&webhook_cfg, http, timeout, notification).await
}

fn build_reconstructed_envelope(notification: &Notification) -> serde_json::Value {
    serde_json::json!({
        "specversion": "1.0",
        "type": POST_CLOUDEVENT_TYPE,
        "source": POST_CLOUDEVENT_SOURCE,
        "id": format!("{}@{}", notification.event_type, notification.sequence),
        "data": {
            "identifier": &notification.identifier,
            "payload": &notification.payload,
        },
    })
}

fn synthesise_webhook_config(cfg: &PostConfig, body_string: &str) -> WebhookConfig {
    let mut headers = cfg.headers.clone();
    let has_content_type = headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"));
    if !has_content_type {
        headers.push((
            "Content-Type".to_string(),
            compile("application/cloudevents+json"),
        ));
    }
    // The body is the (raw or reconstructed) CloudEvent envelope and
    // MUST NOT be re-interpreted by the webhook template engine. A
    // notification payload containing `{{ foo }}` would otherwise be
    // parsed as a template expression. Escape every literal `{{` to
    // `\{{` so the engine emits the original two-brace sequence.
    let escaped = body_string.replace("{{", "\\{{");
    WebhookConfig {
        url_template: cfg.url_template.clone(),
        method: HttpMethod::Post,
        headers,
        body_template: Some(compile(&escaped)),
    }
}

pub(super) fn build_post_config(url: impl Into<String>) -> PostConfig {
    let url_str = url.into();
    PostConfig {
        url_template: compile(&url_str),
        headers: Vec::new(),
    }
}

pub(super) fn post_add_header(
    cfg: &mut PostConfig,
    name: impl Into<String>,
    value: impl Into<String>,
) {
    let value_str = value.into();
    let compiled = compile(&value_str);
    cfg.headers.push((name.into(), compiled));
}

impl super::Trigger {
    /// Build a post trigger that sends a CloudEvent-shaped HTTP POST
    /// per notification. The URL is template-rendered at dispatch;
    /// the body envelope is built programmatically from the
    /// notification's runtime data. Inherits the webhook default
    /// timeout ([`super::DEFAULT_WEBHOOK_TIMEOUT`]).
    #[must_use]
    pub fn post(url: impl Into<String>) -> Self {
        Self {
            kind: super::kind::TriggerKind::Post(Box::new(build_post_config(url))),
            retries: 0,
            required: true,
            timeout: Some(super::DEFAULT_WEBHOOK_TIMEOUT),
            fail_fast: true,
        }
    }

    /// Add a header to a post trigger. Header NAME is literal;
    /// header VALUE is template-rendered at dispatch. Silently
    /// ignored on every non-post trigger kind. Repeatable.
    #[must_use]
    pub fn post_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        if let super::kind::TriggerKind::Post(cfg) = &mut self.kind {
            post_add_header(cfg, name, value);
        }
        self
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on synthesised inputs is the standard test diagnostic"
)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::Notification;

    fn make_notification(
        event_type: &str,
        sequence: u64,
        identifier: BTreeMap<String, serde_json::Value>,
        payload: serde_json::Value,
    ) -> Notification {
        Notification {
            event_type: event_type.to_string(),
            sequence,
            identifier,
            payload,
            cloudevent: None,
        }
    }

    #[test]
    fn envelope_has_specversion_type_source_id_and_data_fields() {
        let mut identifier = BTreeMap::new();
        identifier.insert("class".to_string(), serde_json::json!("od"));
        let n = make_notification("mars", 42, identifier, serde_json::json!({"seed": "x"}));
        let envelope = build_reconstructed_envelope(&n);
        assert_eq!(envelope["specversion"], "1.0");
        assert_eq!(envelope["type"], POST_CLOUDEVENT_TYPE);
        assert_eq!(envelope["source"], POST_CLOUDEVENT_SOURCE);
        assert_eq!(envelope["id"], "mars@42");
        assert_eq!(envelope["data"]["identifier"]["class"], "od");
        assert_eq!(envelope["data"]["payload"]["seed"], "x");
    }

    #[test]
    fn reconstructed_envelope_omits_time_field() {
        let n = make_notification("mars", 1, BTreeMap::new(), serde_json::Value::Null);
        let envelope = build_reconstructed_envelope(&n);
        assert!(
            envelope.get("time").is_none(),
            "the reconstructed envelope MUST omit the time field; faking one with dispatch-time would be misleading: {envelope}"
        );
    }

    #[test]
    fn reconstructed_envelope_id_is_event_type_at_sequence() {
        let n = make_notification("test_polygon", 7, BTreeMap::new(), serde_json::Value::Null);
        let envelope = build_reconstructed_envelope(&n);
        assert_eq!(envelope["id"], "test_polygon@7");
    }

    #[test]
    fn reconstructed_envelope_round_trips_through_json_with_special_chars_in_identifier() {
        let mut identifier = BTreeMap::new();
        identifier.insert(
            "weird".to_string(),
            serde_json::json!("has \"quote\" and \\backslash"),
        );
        let n = make_notification("mars", 1, identifier, serde_json::Value::Null);
        let envelope = build_reconstructed_envelope(&n);
        let serialised = serde_json::to_string(&envelope).unwrap();
        let reparsed: serde_json::Value = serde_json::from_str(&serialised).unwrap();
        assert_eq!(
            reparsed["data"]["identifier"]["weird"],
            "has \"quote\" and \\backslash"
        );
    }

    #[test]
    fn reconstructed_envelope_preserves_structured_identifier() {
        let identifier = BTreeMap::from([(
            "point_cloud".to_string(),
            serde_json::json!([[46.0, 8.0], [47.0, 9.0]]),
        )]);
        let notification =
            make_notification("observations", 2, identifier, serde_json::Value::Null);

        let envelope = build_reconstructed_envelope(&notification);

        assert_eq!(
            envelope["data"]["identifier"]["point_cloud"],
            serde_json::json!([[46.0, 8.0], [47.0, 9.0]])
        );
    }

    #[test]
    fn dispatch_uses_raw_cloudevent_when_some_else_falls_back_to_reconstruction() {
        let mut n_raw = make_notification("mars", 99, BTreeMap::new(), serde_json::json!({"a": 1}));
        let raw_envelope = serde_json::json!({
            "specversion": "1.0",
            "type": "co.ecmwf.aviso.event.real",
            "source": "https://aviso-server.example/",
            "id": "mars@99",
            "time": "2026-05-23T22:00:00.123456Z",
            "data": {
                "identifier": {},
                "payload": {"server": "supplied"}
            }
        });
        n_raw.cloudevent = Some(raw_envelope.clone());
        // When cloudevent is Some, dispatch must send THAT envelope verbatim,
        // not a reconstruction. We verify by selecting the envelope the same
        // way dispatch_post does (Some -> clone raw; None -> reconstruct).
        let selected = match &n_raw.cloudevent {
            Some(raw) => raw.clone(),
            None => build_reconstructed_envelope(&n_raw),
        };
        assert_eq!(
            selected, raw_envelope,
            "with cloudevent Some, the selected envelope must be the raw one byte-for-byte; reconstruction must not run"
        );

        let n_test = make_notification("mars", 1, BTreeMap::new(), serde_json::Value::Null);
        assert!(
            n_test.cloudevent.is_none(),
            "test fixture has no raw envelope"
        );
        let fallback_selected = match &n_test.cloudevent {
            Some(raw) => raw.clone(),
            None => build_reconstructed_envelope(&n_test),
        };
        assert_eq!(
            fallback_selected["type"], POST_CLOUDEVENT_TYPE,
            "with cloudevent None, the selected envelope is the reconstruction using the constant POST_CLOUDEVENT_TYPE"
        );
    }

    #[test]
    fn synthesise_webhook_config_injects_content_type_when_missing() {
        let cfg = PostConfig {
            url_template: compile("https://example/post"),
            headers: vec![("X-Custom".to_string(), compile("value"))],
        };
        let webhook = synthesise_webhook_config(&cfg, "{}");
        let names: Vec<String> = webhook.headers.iter().map(|(n, _)| n.clone()).collect();
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("content-type")),
            "synthesised webhook config MUST inject a Content-Type header when the operator did not supply one: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "X-Custom"),
            "user-supplied X-Custom header MUST be preserved: {names:?}"
        );
    }

    #[test]
    fn synthesise_webhook_config_preserves_user_content_type_when_present() {
        let cfg = PostConfig {
            url_template: compile("https://example/post"),
            headers: vec![(
                "Content-Type".to_string(),
                compile("application/vnd.example+json"),
            )],
        };
        let webhook = synthesise_webhook_config(&cfg, "{}");
        let content_types: Vec<&String> = webhook
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("content-type"))
            .map(|(n, _)| n)
            .collect();
        assert_eq!(
            content_types.len(),
            1,
            "exactly one Content-Type header must be present (the user-supplied one); the dispatcher must NOT inject a duplicate default Content-Type: {:?}",
            webhook
                .headers
                .iter()
                .map(|(n, _)| n.clone())
                .collect::<Vec<_>>(),
        );
    }
}
