// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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
    Sink, TemplateError, TemplateErrorKind, compile, render_value, template_error_to_trigger_error,
};
use crate::Notification;

fn make_notification() -> Notification {
    let mut identifier = BTreeMap::new();
    identifier.insert("country".to_string(), serde_json::json!("uk"));
    identifier.insert(
        "point_cloud".to_string(),
        serde_json::json!([[46.0, 8.0], [47.0, 9.0]]),
    );
    Notification {
        event_type: "mars".to_string(),
        sequence: 42,
        identifier,
        payload: serde_json::json!({ "location": "south", "qty": 7 }),
        cloudevent: None,
    }
}

#[test]
fn compile_empty_template_yields_no_segments() {
    let t = compile("").expect("empty template must compile");
    let out = t
        .render(&make_notification(), Sink::Raw)
        .expect("render empty");
    assert_eq!(out, "");
}

#[test]
fn compile_literal_only_returns_unchanged() {
    let t = compile("hello world").expect("compile literal");
    let out = t
        .render(&make_notification(), Sink::Raw)
        .expect("render literal");
    assert_eq!(out, "hello world");
}

#[test]
fn compile_notification_event_type_substitutes_unquoted() {
    let t = compile("event: {{ notification.event_type }}").expect("compile");
    let out = t.render(&make_notification(), Sink::Raw).expect("render");
    assert_eq!(out, "event: mars");
}

#[test]
fn compile_notification_sequence_substitutes_as_number() {
    let t = compile("seq={{ notification.sequence }}").expect("compile");
    let out = t.render(&make_notification(), Sink::Raw).expect("render");
    assert_eq!(out, "seq=42");
}

#[test]
fn compile_notification_nested_identifier_path() {
    let t = compile("country: {{ notification.identifier.country }}").expect("compile");
    let out = t.render(&make_notification(), Sink::Raw).expect("render");
    assert_eq!(out, "country: uk");
}

#[test]
fn compile_structured_identifier_renders_compact_json() {
    let template = compile("{{ notification.identifier.point_cloud }}").expect("compile");
    let output = template
        .render(&make_notification(), Sink::Raw)
        .expect("render");
    assert_eq!(output, "[[46.0,8.0],[47.0,9.0]]");
}

#[test]
fn compile_notification_payload_object_renders_as_compact_json() {
    let t = compile("body={{ notification.payload }}").expect("compile");
    let out = t.render(&make_notification(), Sink::Raw).expect("render");
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
    let out = t.render(&make_notification(), Sink::Raw).expect("render");
    assert_eq!(out, "loc:south");
}

#[test]
fn compile_whole_notification_renders_as_compact_json() {
    let t = compile("{{ notification }}").expect("compile");
    let out = t.render(&make_notification(), Sink::Raw).expect("render");
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
    let err = t
        .render(&make_notification(), Sink::Raw)
        .expect_err("must miss");
    assert_eq!(err.kind, TemplateErrorKind::Missing);
    assert_eq!(err.field, "notification.payload.nope");
}

#[test]
fn escape_double_brace_renders_as_literal() {
    let t = compile("literal: \\{{ inside }}").expect("compile");
    let out = t.render(&make_notification(), Sink::Raw).expect("render");
    assert_eq!(out, "literal: {{ inside }}");
}

#[test]
fn render_env_variable_set_substitutes_value() {
    let mut env: HashMap<&str, &str> = HashMap::new();
    env.insert("MY_TOKEN", "hello");
    let t = compile("env=\"{{ env.MY_TOKEN }}\"").expect("compile");
    let out = t
        .render_with_env(
            &make_notification(),
            |name| {
                env.get(name)
                    .map(|v| (*v).to_string())
                    .ok_or(TemplateErrorKind::EnvNotSet)
            },
            Sink::Raw,
        )
        .expect("render");
    assert_eq!(out, "env=\"hello\"");
}

#[test]
fn render_env_variable_missing_returns_envnotset() {
    let t = compile("{{ env.NOT_SET_BY_TEST }}").expect("compile");
    let err = t
        .render_with_env(
            &make_notification(),
            |_name| Err(TemplateErrorKind::EnvNotSet),
            Sink::Raw,
        )
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
        .render_with_env(
            &make_notification(),
            |_name| Err(TemplateErrorKind::EnvNotUnicode),
            Sink::Raw,
        )
        .expect_err("must report not-unicode");
    assert_eq!(err.kind, TemplateErrorKind::EnvNotUnicode);
    assert_eq!(err.field, "NOT_UTF8");
}

#[test]
fn render_falls_back_to_process_env_when_not_using_seam() {
    let t = compile("{{ env.PATH }}").expect("compile");
    let out = t.render(&make_notification(), Sink::Raw).expect("render");
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
        let out = t
            .render(&make_notification(), Sink::Raw)
            .expect("literal render");
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

#[test]
fn shell_sink_quotes_each_value_for_the_context_it_lands_in() {
    let mut identifier = BTreeMap::new();
    identifier.insert("country".to_string(), serde_json::json!("a'b$(x)"));
    let n = Notification {
        event_type: "mars".to_string(),
        sequence: 1,
        identifier,
        payload: serde_json::json!({}),
        cloudevent: None,
    };
    let render = |template: &str| compile(template).unwrap().render(&n, Sink::Shell).unwrap();
    assert_eq!(
        render("run {{ notification.identifier.country }}"),
        "run 'a'\\''b$(x)'"
    );
    assert_eq!(
        render("run '{{ notification.identifier.country }}'"),
        "run 'a'\\''b$(x)'"
    );
    assert_eq!(
        render("run \"{{ notification.identifier.country }}\""),
        "run \"a'b\\$(x)\""
    );
    // Numbers and event types are quoted the same way; the shell sees
    // one word either way.
    assert_eq!(
        render("run {{ notification.sequence }} {{ notification.event_type }}"),
        "run '1' 'mars'"
    );
}

#[test]
fn url_sink_percent_encodes_values() {
    let t =
        compile("https://h/{{ notification.event_type }}?p={{ notification.payload }}").unwrap();
    let mut n = make_notification();
    n.event_type = "a/b".to_string();
    let out = t.render(&n, Sink::Url).unwrap();
    assert!(out.starts_with("https://h/a%2Fb?p=%7B"), "got: {out}");
    assert!(!out.contains('"'), "got: {out}");
}

#[test]
fn url_sink_refuses_a_value_in_the_scheme_or_authority() {
    let n = make_notification();
    let refuse = |template: &str| {
        let err = compile(template)
            .unwrap()
            .render(&n, Sink::Url)
            .unwrap_err();
        assert_eq!(
            err.kind,
            TemplateErrorKind::ValueInUrlAuthority,
            "{template}"
        );
        err.field
    };
    assert_eq!(
        refuse("https://{{ notification.event_type }}/hook"),
        "notification.event_type"
    );
    assert_eq!(
        refuse("https://h:{{ notification.sequence }}/hook"),
        "notification.sequence"
    );
    assert_eq!(
        refuse("{{ notification.identifier.country }}://h/hook"),
        "notification.identifier.country"
    );
    assert_eq!(refuse("{{ notification }}"), "notification");
    // Once the path has started, values are fine, including after an
    // env value that supplied the host.
    let t = compile("{{ env.BASE }}/{{ notification.event_type }}?s={{ notification.sequence }}")
        .unwrap();
    let out = t
        .render_with_env(&n, |_| Ok("https://hooks.example".to_string()), Sink::Url)
        .unwrap();
    assert_eq!(out, "https://hooks.example/mars?s=42");
    // A host that comes only from an env value, with the value in the
    // query, is also fine.
    let t = compile("{{ env.BASE }}?s={{ notification.sequence }}").unwrap();
    let out = t
        .render_with_env(&n, |_| Ok("https://hooks.example".to_string()), Sink::Url)
        .unwrap();
    assert_eq!(out, "https://hooks.example?s=42");
}

#[test]
fn raw_sink_inserts_values_verbatim() {
    let t = compile("{{ notification.event_type }}").unwrap();
    let mut n = make_notification();
    n.event_type = "a/b'c\"d".to_string();
    assert_eq!(t.render(&n, Sink::Raw).unwrap(), "a/b'c\"d");
}
