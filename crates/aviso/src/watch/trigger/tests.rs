//! Unit tests for the trigger module's public surface: `Trigger`
//! builder defaults and setters, `Debug`/`Clone` shape,
//! `TriggerKindLabel` `Display`, `TriggerError` variant carriers
//! (including the load-bearing redaction discipline on
//! `TriggerError::Template`).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap on constructor success and panic on unexpected variant are the standard test diagnostics"
)]

use std::path::PathBuf;

use super::{Trigger, TriggerError, TriggerKindLabel};
use crate::watch::trigger::kind::TriggerKind;

#[test]
fn echo_constructor_uses_default_retries_zero_and_required_true() {
    let trigger = Trigger::echo();
    assert!(matches!(trigger.kind, TriggerKind::Echo));
    assert_eq!(trigger.retries, 0);
    assert!(trigger.required);
}

#[test]
fn log_constructor_uses_default_retries_zero_and_required_true() {
    let trigger = Trigger::log("/tmp/some.log");
    let TriggerKind::Log { path } = &trigger.kind else {
        panic!("expected Log variant");
    };
    assert_eq!(path, &PathBuf::from("/tmp/some.log"));
    assert_eq!(trigger.retries, 0);
    assert!(trigger.required);
}

#[test]
fn retries_setter_overrides_default() {
    let trigger = Trigger::echo().retries(7);
    assert_eq!(trigger.retries, 7);
    assert!(matches!(trigger.kind, TriggerKind::Echo));
    assert!(trigger.required);
}

#[test]
fn required_setter_overrides_default() {
    let trigger = Trigger::echo().required(false);
    assert!(!trigger.required);
    assert_eq!(trigger.retries, 0);
}

#[test]
fn trigger_clone_preserves_all_fields() {
    let original = Trigger::log("/tmp/clone.log").retries(3).required(false);
    let cloned = original.clone();
    let (TriggerKind::Log { path: a }, TriggerKind::Log { path: b }) =
        (&original.kind, &cloned.kind)
    else {
        panic!("clone did not preserve Log variant");
    };
    assert_eq!(a, b);
    assert_eq!(cloned.retries, original.retries);
    assert_eq!(cloned.required, original.required);
}

#[test]
fn trigger_debug_includes_all_fields() {
    let trigger = Trigger::log("/tmp/dbg.log").retries(2).required(true);
    let dbg = format!("{trigger:?}");
    assert!(dbg.contains("Trigger"), "got: {dbg}");
    assert!(dbg.contains("kind"), "got: {dbg}");
    assert!(dbg.contains("retries"), "got: {dbg}");
    assert!(dbg.contains("required"), "got: {dbg}");
    assert!(dbg.contains("/tmp/dbg.log"), "got: {dbg}");
}

#[test]
fn trigger_kind_label_display_for_echo() {
    let label = TriggerKindLabel::Echo;
    assert_eq!(label.to_string(), "echo");
}

#[test]
fn trigger_kind_label_display_for_log_includes_path() {
    let label = TriggerKindLabel::Log {
        path: PathBuf::from("/var/log/aviso.log"),
    };
    assert_eq!(label.to_string(), "log(/var/log/aviso.log)");
}

#[test]
fn trigger_kind_label_display_for_command_is_bare() {
    let label = TriggerKindLabel::Command;
    assert_eq!(label.to_string(), "command");
}

#[test]
fn trigger_error_io_variant_carries_io_kind() {
    let err: TriggerError = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe").into();
    match err {
        TriggerError::Io(inner) => assert_eq!(inner.kind(), std::io::ErrorKind::BrokenPipe),
        other => panic!("expected Io, got {other:?}"),
    }
}

#[test]
fn trigger_error_encode_variant_carries_serde_error() {
    let parse_err = serde_json::from_str::<i32>("not a number").unwrap_err();
    let err: TriggerError = parse_err.into();
    assert!(matches!(err, TriggerError::Encode(_)));
}

#[test]
fn trigger_error_template_display_uses_safe_context_not_raw_template() {
    use crate::watch::TemplateErrorKind;
    let err = TriggerError::Template {
        context: "command".to_string(),
        field: "notification.payload.target".to_string(),
        kind: TemplateErrorKind::Missing,
    };
    let rendered = err.to_string();
    assert!(rendered.contains("command"), "got: {rendered}");
    assert!(
        rendered.contains("notification.payload.target"),
        "got: {rendered}"
    );
    assert!(rendered.contains("Missing"), "got: {rendered}");
}
