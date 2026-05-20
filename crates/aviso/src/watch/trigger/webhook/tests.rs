//! Unit tests for the webhook trigger: redaction discipline, default
//! and overridden HTTP-status handling, template substitution in URL
//! / header values / body, timeout, transport errors, and the
//! `render_webhook_parts_with_env` test seam.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect on dispatch results and panic on unexpected variants are the standard test diagnostics"
)]

use std::collections::BTreeMap;
use std::time::Duration;

use wiremock::matchers::{body_string, header, method as wm_method, path as wm_path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{
    HttpMethod, RING_CAP, RenderedWebhook, build_webhook_config, dispatch_webhook,
    render_webhook_parts_with_env, webhook_add_header, webhook_set_body_template,
    webhook_set_method,
};
use crate::Notification;
use crate::watch::TriggerError;
use crate::watch::trigger::TemplateErrorKind;

fn make_notification() -> Notification {
    let mut identifier = BTreeMap::new();
    identifier.insert("country".to_string(), "uk".to_string());
    Notification {
        event_type: "mars".to_string(),
        sequence: 42,
        identifier,
        payload: serde_json::json!({ "location": "south", "qty": 7 }),
        request_id: Some("req-abc".to_string()),
    }
}

fn no_env_resolver() -> impl Fn(&str) -> Result<String, TemplateErrorKind> {
    |_name| Err(TemplateErrorKind::EnvNotSet)
}

#[test]
fn webhook_config_debug_redacts_url_header_values_and_body() {
    let mut cfg = build_webhook_config("https://hooks.slack.com/services/SECRET_TOKEN");
    webhook_add_header(&mut cfg, "Authorization", "Bearer LEAKED_TOKEN_VALUE");
    webhook_add_header(&mut cfg, "X-Custom", "another-secret-value");
    webhook_set_body_template(&mut cfg, r#"{"secret_body_marker": "SHOULD_NOT_LEAK"}"#);
    let rendered = format!("{cfg:?}");
    assert!(
        !rendered.contains("SECRET_TOKEN"),
        "URL template must NOT leak into Debug: {rendered}"
    );
    assert!(
        !rendered.contains("LEAKED_TOKEN_VALUE"),
        "Authorization header value must NOT leak into Debug: {rendered}"
    );
    assert!(
        !rendered.contains("another-secret-value"),
        "header values must NOT leak into Debug: {rendered}"
    );
    assert!(
        !rendered.contains("SHOULD_NOT_LEAK"),
        "body template must NOT leak into Debug: {rendered}"
    );
    assert!(
        rendered.contains("header_count: 2"),
        "Debug must surface header_count: 2 as a structural fact: {rendered}"
    );
}

#[tokio::test]
async fn webhook_dispatch_2xx_returns_ok() {
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!("{}/hook", server.uri()));
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[tokio::test]
async fn webhook_dispatch_5xx_returns_webhook_error_with_body_tail() {
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal failure"))
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!("{}/hook", server.uri()));
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    match result {
        Err(TriggerError::Webhook { status, body_tail }) => {
            assert_eq!(status.map(|s| s.as_u16()), Some(500));
            assert!(
                body_tail.contains("internal failure"),
                "got body_tail: {body_tail}"
            );
        }
        other => panic!("expected Webhook error, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_dispatch_4xx_returns_webhook_error() {
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!("{}/hook", server.uri()));
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    match result {
        Err(TriggerError::Webhook { status, body_tail }) => {
            assert_eq!(status.map(|s| s.as_u16()), Some(400));
            assert!(body_tail.contains("bad request"), "got: {body_tail}");
        }
        other => panic!("expected Webhook error, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_dispatch_invalid_url_returns_webhook_build_error() {
    // A URL that cannot be parsed (whitespace + no scheme) is
    // accepted by the template engine as a literal string but
    // rejected by reqwest at build time with is_builder() = true.
    // The dispatcher must surface this as TriggerError::WebhookBuild
    // (terminal under fail_fast = true), NOT as a transport-error
    // Webhook { status: None } (which would waste the retry budget
    // on a deterministic misconfiguration).
    let cfg = build_webhook_config("not a valid url");
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    match result {
        Err(TriggerError::WebhookBuild { reason }) => {
            assert!(!reason.is_empty(), "WebhookBuild reason must be non-empty");
            // Reason MUST NOT include the rendered URL: reqwest's
            // Display includes the URL in builder errors, and the
            // URL can carry secrets.
            assert!(
                !reason.contains("not a valid url"),
                "WebhookBuild reason must not leak the rendered URL; got: {reason}"
            );
        }
        other => panic!("expected WebhookBuild for invalid URL, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_dispatch_transport_error_returns_none_status() {
    // Refused connection: bind to a port the kernel will refuse.
    // 127.0.0.1:1 is reliably closed on every test platform; any
    // attempt produces a connect-refused transport error.
    let cfg = build_webhook_config("http://127.0.0.1:1/hook");
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    match result {
        Err(TriggerError::Webhook { status, body_tail }) => {
            assert!(status.is_none(), "got status: {status:?}");
            assert!(
                body_tail.is_empty(),
                "transport error body_tail must be empty: {body_tail}"
            );
        }
        other => panic!("expected Webhook transport error, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_dispatch_timeout_returns_timeout_error() {
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!("{}/hook", server.uri()));
    let http = reqwest::Client::new();
    let started = std::time::Instant::now();
    let result = dispatch_webhook(
        &cfg,
        &http,
        Some(Duration::from_millis(100)),
        &make_notification(),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "timeout must fire quickly; elapsed: {elapsed:?}"
    );
    match result {
        Err(TriggerError::Timeout(t)) => {
            assert_eq!(t, Duration::from_millis(100));
        }
        other => panic!("expected Timeout error, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_dispatch_renders_url_template_at_dispatch() {
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook/mars/42"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!(
        "{}/hook/{{{{ notification.event_type }}}}/{{{{ notification.sequence }}}}",
        server.uri()
    ));
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[tokio::test]
async fn webhook_dispatch_renders_body_template() {
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .and(body_string(r#"{"seq": 42}"#))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut cfg = build_webhook_config(format!("{}/hook", server.uri()));
    webhook_set_body_template(&mut cfg, r#"{"seq": {{ notification.sequence }}}"#);
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[tokio::test]
async fn webhook_dispatch_default_body_is_compact_notification_json() {
    let expected_body = serde_json::to_string(&make_notification()).unwrap();
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .and(body_string(expected_body))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!("{}/hook", server.uri()));
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[tokio::test]
async fn webhook_dispatch_default_content_type_is_application_json() {
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!("{}/hook", server.uri()));
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[tokio::test]
async fn webhook_dispatch_user_content_type_overrides_default() {
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .and(header("content-type", "text/plain"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut cfg = build_webhook_config(format!("{}/hook", server.uri()));
    webhook_add_header(&mut cfg, "Content-Type", "text/plain");
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[tokio::test]
async fn webhook_dispatch_uses_user_method() {
    let server = MockServer::start().await;
    Mock::given(wm_method("PUT"))
        .and(wm_path("/hook"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let mut cfg = build_webhook_config(format!("{}/hook", server.uri()));
    webhook_set_method(&mut cfg, HttpMethod::Put);
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[test]
fn render_webhook_parts_with_env_resolves_env_var_in_header_value() {
    let mut cfg = build_webhook_config("https://example.com/hook");
    webhook_add_header(&mut cfg, "Authorization", "Bearer {{ env.HOOK_TOKEN }}");
    let env_resolver = |name: &str| {
        if name == "HOOK_TOKEN" {
            Ok("test-token-value".to_string())
        } else {
            Err(TemplateErrorKind::EnvNotSet)
        }
    };
    let rendered = render_webhook_parts_with_env(&cfg, &make_notification(), env_resolver)
        .expect("render must succeed when env var is provided");
    let RenderedWebhook { headers, .. } = rendered;
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].0, "Authorization");
    assert_eq!(headers[0].1, "Bearer test-token-value");
}

#[test]
fn render_webhook_parts_with_env_missing_notification_path_in_url_returns_template_error() {
    let cfg = build_webhook_config("https://example.com/{{ notification.nonexistent }}");
    let result = render_webhook_parts_with_env(&cfg, &make_notification(), no_env_resolver());
    match result {
        Err(TriggerError::Template {
            context,
            field,
            kind,
        }) => {
            assert_eq!(context, "webhook url");
            assert_eq!(field, "notification.nonexistent");
            assert_eq!(kind, TemplateErrorKind::Missing);
        }
        other => panic!("expected Template error, got {other:?}"),
    }
}

#[test]
fn render_webhook_parts_with_env_missing_env_var_in_header_returns_template_error() {
    let mut cfg = build_webhook_config("https://example.com/hook");
    webhook_add_header(&mut cfg, "X-Auth", "{{ env.NOT_SET }}");
    let result = render_webhook_parts_with_env(&cfg, &make_notification(), no_env_resolver());
    match result {
        Err(TriggerError::Template {
            context,
            field,
            kind,
        }) => {
            assert_eq!(context, "webhook header");
            assert_eq!(field, "NOT_SET");
            assert_eq!(kind, TemplateErrorKind::EnvNotSet);
        }
        other => panic!("expected Template error, got {other:?}"),
    }
}

#[test]
fn render_webhook_parts_with_env_missing_env_var_in_body_returns_template_error() {
    let mut cfg = build_webhook_config("https://example.com/hook");
    webhook_set_body_template(&mut cfg, r#"{"token": "{{ env.MISSING_BODY_VAR }}"}"#);
    let result = render_webhook_parts_with_env(&cfg, &make_notification(), no_env_resolver());
    match result {
        Err(TriggerError::Template {
            context,
            field,
            kind,
        }) => {
            assert_eq!(context, "webhook body");
            assert_eq!(field, "MISSING_BODY_VAR");
            assert_eq!(kind, TemplateErrorKind::EnvNotSet);
        }
        other => panic!("expected Template error, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_dispatch_body_drain_timeout_returns_timeout_error() {
    // Headers arrive fast, but the body stalls past the per-trigger
    // timeout. Without explicit handling, the dispatcher would
    // truncate the body and classify by the (200) response status,
    // hiding the timeout. capture_body_tail must propagate
    // BodyDrain::Timeout up so the dispatcher returns
    // TriggerError::Timeout(t).
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("never finishes")
                .set_delay(Duration::from_secs(10)),
        )
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!("{}/hook", server.uri()));
    let http = reqwest::Client::new();
    let started = std::time::Instant::now();
    let result = dispatch_webhook(
        &cfg,
        &http,
        Some(Duration::from_millis(150)),
        &make_notification(),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "timeout must fire during body drain too; elapsed: {elapsed:?}"
    );
    match result {
        Err(TriggerError::Timeout(t)) => {
            assert_eq!(t, Duration::from_millis(150));
        }
        other => panic!("expected Timeout from body-drain stall, got {other:?}"),
    }
}

#[tokio::test]
async fn webhook_dispatch_body_tail_capped_at_4kib() {
    // Emit 8 KiB of 'A' followed by a sentinel suffix, then return 500
    // so the dispatcher surfaces the body_tail. The captured tail must
    // be exactly 4096 bytes and must contain the sentinel (i.e., we
    // kept the TAIL, not the head). The body is set as a static
    // string on the response template.
    let mut big_body = "A".repeat(RING_CAP * 2);
    big_body.push_str("SENTINEL");
    let server = MockServer::start().await;
    Mock::given(wm_method("POST"))
        .and(wm_path("/hook"))
        .respond_with(ResponseTemplate::new(500).set_body_string(big_body.clone()))
        .mount(&server)
        .await;
    let cfg = build_webhook_config(format!("{}/hook", server.uri()));
    let http = reqwest::Client::new();
    let result = dispatch_webhook(&cfg, &http, None, &make_notification()).await;
    match result {
        Err(TriggerError::Webhook { status, body_tail }) => {
            assert_eq!(status.map(|s| s.as_u16()), Some(500));
            assert_eq!(
                body_tail.len(),
                RING_CAP,
                "ring buffer must cap at RING_CAP bytes"
            );
            assert!(
                body_tail.ends_with("SENTINEL"),
                "ring must keep the TAIL, got last 32 chars: {:?}",
                &body_tail[body_tail.len().saturating_sub(32)..]
            );
        }
        other => panic!("expected Webhook 500, got {other:?}"),
    }
}
