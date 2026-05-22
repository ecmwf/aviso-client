//! Unit tests for the template engine: parser edge cases, resolver
//! rules, and the redaction discipline (raw template stays out of the
//! public `TriggerError::Template` variant).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect on engine output and panic on unexpected variant are the standard test diagnostics"
)]

use std::collections::BTreeMap;
use std::collections::HashMap;

use super::{
    TemplateError, TemplateErrorKind, compile, render_value, template_error_to_trigger_error,
};
use crate::Notification;

fn make_notification() -> Notification {
    let mut identifier = BTreeMap::new();
    identifier.insert("country".to_string(), "uk".to_string());
    Notification {
        event_type: "mars".to_string(),
        sequence: 42,
        identifier,
        payload: serde_json::json!({ "location": "south", "qty": 7 }),
    }
}

#[test]
fn compile_empty_template_yields_no_segments() {
    let t = compile("").expect("empty template must compile");
    let out = t.render(&make_notification()).expect("render empty");
    assert_eq!(out, "");
}

#[test]
fn compile_literal_only_returns_unchanged() {
    let t = compile("hello world").expect("compile literal");
    let out = t.render(&make_notification()).expect("render literal");
    assert_eq!(out, "hello world");
}

#[test]
fn compile_notification_event_type_substitutes_unquoted() {
    let t = compile("event: {{ notification.event_type }}").expect("compile");
    let out = t.render(&make_notification()).expect("render");
    assert_eq!(out, "event: mars");
}

#[test]
fn compile_notification_sequence_substitutes_as_number() {
    let t = compile("seq={{ notification.sequence }}").expect("compile");
    let out = t.render(&make_notification()).expect("render");
    assert_eq!(out, "seq=42");
}

#[test]
fn compile_notification_nested_identifier_path() {
    let t = compile("country: {{ notification.identifier.country }}").expect("compile");
    let out = t.render(&make_notification()).expect("render");
    assert_eq!(out, "country: uk");
}

#[test]
fn compile_notification_payload_object_renders_as_compact_json() {
    let t = compile("body={{ notification.payload }}").expect("compile");
    let out = t.render(&make_notification()).expect("render");
    // Object renders as compact JSON; key order follows serde_json
    // (insertion order; we control insertion via the make_notification
    // helper).
    assert!(out.starts_with("body={"), "got: {out}");
    assert!(out.contains("\"location\":\"south\""), "got: {out}");
    assert!(out.contains("\"qty\":7"), "got: {out}");
}

#[test]
fn compile_notification_payload_string_field_renders_unquoted() {
    let t = compile("loc:{{ notification.payload.location }}").expect("compile");
    let out = t.render(&make_notification()).expect("render");
    assert_eq!(out, "loc:south");
}

#[test]
fn compile_whole_notification_renders_as_compact_json() {
    let t = compile("{{ notification }}").expect("compile");
    let out = t.render(&make_notification()).expect("render");
    assert!(out.starts_with('{'), "got: {out}");
    assert!(out.contains("\"event_type\":\"mars\""), "got: {out}");
}

#[test]
fn compile_unclosed_braces_returns_bad_syntax_with_safe_label() {
    let err = compile("hello {{ notification.foo").expect_err("must reject unclosed");
    assert_eq!(err.kind, TemplateErrorKind::BadSyntax);
    assert_eq!(err.field, "unclosed_braces");
    // Raw template is retained but the public field is a safe static
    // label; this is the redaction discipline that prevents secrets in
    // unclosed expressions from leaking.
    assert_eq!(err.raw_template, "hello {{ notification.foo");
}

#[test]
fn compile_empty_path_segment_returns_bad_syntax() {
    let err = compile("{{ notification.a..b }}").expect_err("must reject empty segment");
    assert_eq!(err.kind, TemplateErrorKind::BadSyntax);
    assert_eq!(err.field, "empty_path_segment");
}

#[test]
fn compile_unknown_namespace_returns_bad_syntax() {
    let err = compile("{{ unknown.field }}").expect_err("must reject unknown namespace");
    assert_eq!(err.kind, TemplateErrorKind::BadSyntax);
    assert_eq!(err.field, "unknown_namespace");
}

#[test]
fn compile_escape_inside_expression_returns_bad_syntax() {
    let err = compile("{{ \\notification }}").expect_err("must reject escape inside expr");
    assert_eq!(err.kind, TemplateErrorKind::BadSyntax);
    assert_eq!(err.field, "escape_inside_expr");
}

#[test]
fn render_missing_path_returns_missing_with_path_field() {
    let t = compile("{{ notification.payload.nope }}").expect("compile");
    let err = t.render(&make_notification()).expect_err("must miss");
    assert_eq!(err.kind, TemplateErrorKind::Missing);
    assert_eq!(err.field, "notification.payload.nope");
}

#[test]
fn escape_double_brace_renders_as_literal() {
    let t = compile("literal: \\{{ inside }}").expect("compile");
    let out = t.render(&make_notification()).expect("render");
    assert_eq!(out, "literal: {{ inside }}");
}

#[test]
fn render_env_variable_set_substitutes_value() {
    let mut env: HashMap<&str, &str> = HashMap::new();
    env.insert("MY_TOKEN", "hello");
    let t = compile("env=\"{{ env.MY_TOKEN }}\"").expect("compile");
    let out = t
        .render_with_env(&make_notification(), |name| {
            env.get(name)
                .map(|v| (*v).to_string())
                .ok_or(TemplateErrorKind::EnvNotSet)
        })
        .expect("render");
    assert_eq!(out, "env=\"hello\"");
}

#[test]
fn render_env_variable_missing_returns_envnotset() {
    let t = compile("{{ env.NOT_SET_BY_TEST }}").expect("compile");
    let err = t
        .render_with_env(&make_notification(), |_name| {
            Err(TemplateErrorKind::EnvNotSet)
        })
        .expect_err("must miss");
    assert_eq!(err.kind, TemplateErrorKind::EnvNotSet);
    assert_eq!(err.field, "NOT_SET_BY_TEST");
}

#[test]
fn template_error_kind_notification_encode_is_distinct_from_missing() {
    // The notification-encode path is practically unreachable (the
    // well-typed Notification struct round-trips through
    // serde_json::to_value), but the variant must exist and be
    // distinct from Missing so an operator who hits the impossible
    // path is pointed at the notification rather than at a missing
    // template path.
    assert_ne!(
        TemplateErrorKind::NotificationEncode,
        TemplateErrorKind::Missing
    );
    let err = TemplateError {
        raw_template: "x".to_string(),
        field: "notification".to_string(),
        kind: TemplateErrorKind::NotificationEncode,
    };
    assert_eq!(err.kind, TemplateErrorKind::NotificationEncode);
}

#[test]
fn render_env_variable_not_unicode_returns_envnotunicode() {
    // The production path maps VarError::NotUnicode -> EnvNotUnicode
    // via the resolver wired up in `render()`. We exercise the
    // distinct error variant through the resolver seam here because
    // the crate forbids `unsafe` and `std::env::set_var` would be
    // required to install a non-UTF-8 env var in the live process.
    let t = compile("{{ env.NOT_UTF8 }}").expect("compile");
    let err = t
        .render_with_env(&make_notification(), |_name| {
            Err(TemplateErrorKind::EnvNotUnicode)
        })
        .expect_err("must report not-unicode");
    assert_eq!(err.kind, TemplateErrorKind::EnvNotUnicode);
    assert_eq!(err.field, "NOT_UTF8");
}

#[test]
fn render_falls_back_to_process_env_when_not_using_seam() {
    let t = compile("{{ env.PATH }}").expect("compile");
    let out = t.render(&make_notification()).expect("render");
    assert!(
        !out.is_empty(),
        "PATH is virtually always set in test environments; if this assertion fires the CI host has no PATH which is suspicious"
    );
}

#[test]
fn render_value_null_yields_literal_null_string() {
    assert_eq!(render_value(&serde_json::Value::Null), "null");
}

#[test]
fn render_value_bool_yields_unquoted_bool() {
    assert_eq!(render_value(&serde_json::Value::Bool(true)), "true");
    assert_eq!(render_value(&serde_json::Value::Bool(false)), "false");
}

#[test]
fn template_error_clone_preserves_all_fields() {
    let e = TemplateError {
        raw_template: "raw".to_string(),
        field: "f".to_string(),
        kind: TemplateErrorKind::Missing,
    };
    let cloned = e.clone();
    assert_eq!(cloned.raw_template, e.raw_template);
    assert_eq!(cloned.field, e.field);
    assert_eq!(cloned.kind, e.kind);
}

#[test]
fn compile_then_render_roundtrip_on_literal_only_templates() {
    // Property-flavour test: any template without `{{` should render
    // as itself for any notification.
    for input in [
        "",
        "x",
        "hello world",
        "no expressions here",
        "punctuation, semicolons; and {single braces}",
    ] {
        let t = compile(input).expect("literal compile");
        let out = t.render(&make_notification()).expect("literal render");
        assert_eq!(out, input);
    }
}

#[test]
fn compiled_template_raw_returns_original_source() {
    let src = "x={{ notification.event_type }}";
    let t = compile(src).expect("compile");
    assert_eq!(t.raw(), src);
}

#[test]
fn template_error_to_trigger_error_omits_raw_template_from_public_variant() {
    let private_err = TemplateError {
        raw_template: "Authorization: Bearer SUPER_SECRET_TOKEN {{ notification.event_type }}"
            .to_string(),
        field: "notification.event_type".to_string(),
        kind: TemplateErrorKind::Missing,
    };
    let public_err = template_error_to_trigger_error(private_err, "webhook header");
    let rendered = public_err.to_string();
    assert!(rendered.contains("webhook header"), "got: {rendered}");
    assert!(
        rendered.contains("notification.event_type"),
        "got: {rendered}"
    );
    assert!(
        !rendered.contains("SUPER_SECRET_TOKEN"),
        "raw template must not leak into public error: {rendered}"
    );
}
