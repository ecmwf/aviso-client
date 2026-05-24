//! Unit tests for the command trigger: the redaction discipline,
//! happy-path dispatch, timeout-and-kill, env-var injection, stderr
//! tail capture, drain-ring shape, and the surface tests for
//! `drain_to_ring` and `normalize_key`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect on dispatch results and panic on unexpected variants are the standard test diagnostics"
)]

use std::collections::BTreeMap;

use super::{build_command_config, dispatch_command, drain_to_ring, normalize_key};
use crate::Notification;
use crate::watch::TriggerError;

fn make_notification() -> Notification {
    let mut identifier = BTreeMap::new();
    identifier.insert("country".to_string(), "uk".to_string());
    Notification {
        event_type: "mars".to_string(),
        sequence: 42,
        identifier,
        payload: serde_json::json!({ "location": "south", "qty": 7 }),
        cloudevent: None,
    }
}

#[test]
fn command_config_debug_redacts_raw_template_and_env_values() {
    let mut cfg = build_command_config(
        "./curl --header 'Authorization: Bearer SUPER_SECRET_TOKEN' https://example.com",
    );
    cfg.env
        .insert("API_TOKEN".to_string(), "leaked-secret-value".to_string());
    cfg.env
        .insert("DB_PASSWORD".to_string(), "another-secret".to_string());
    let rendered = format!("{cfg:?}");
    assert!(
        !rendered.contains("SUPER_SECRET_TOKEN"),
        "raw command template must NOT leak into Debug: {rendered}"
    );
    assert!(
        !rendered.contains("leaked-secret-value"),
        "env-var values must NOT leak into Debug: {rendered}"
    );
    assert!(
        !rendered.contains("another-secret"),
        "env-var values must NOT leak into Debug: {rendered}"
    );
    assert!(
        rendered.contains("env_count: 2"),
        "Debug must surface env_count: 2 as a structural fact (the test set two env vars): {rendered}"
    );
}

#[tokio::test]
async fn command_trigger_zero_exit_is_success() {
    let cfg = build_command_config("true");
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[tokio::test]
async fn command_trigger_nonzero_exit_returns_command_error_with_stderr_tail() {
    let cfg = build_command_config("printf 'bad happened' 1>&2; exit 7");
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    match result {
        Err(TriggerError::Command {
            exit_code,
            stderr_tail,
        }) => {
            assert_eq!(exit_code, 7, "got exit_code: {exit_code}");
            assert!(stderr_tail.contains("bad happened"), "got: {stderr_tail}");
        }
        other => panic!("expected Command error, got {other:?}"),
    }
}

#[tokio::test]
async fn command_trigger_stderr_tail_capped_at_4kib() {
    // Emit 8 KiB of 'A' followed by a sentinel suffix to stderr, then
    // exit 1. The captured tail must be exactly 4096 bytes and must
    // contain the sentinel suffix (i.e., we kept the TAIL, not the head).
    //
    // awk is POSIX-required and avoids depending on perl. The BEGIN
    // block runs once at startup; exit 1 propagates as the shell's
    // exit status because awk is the last (only) command.
    let cfg = build_command_config(
        "awk 'BEGIN { for (i = 0; i < 8192; i++) printf \"A\"; printf \"SENTINEL\"; exit 1 }' 1>&2",
    );
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    match result {
        Err(TriggerError::Command {
            exit_code: 1,
            stderr_tail,
        }) => {
            assert_eq!(
                stderr_tail.len(),
                4096,
                "ring buffer must cap at 4096 bytes"
            );
            assert!(stderr_tail.ends_with("SENTINEL"), "ring must keep the TAIL");
        }
        other => panic!("expected Command exit 1, got {other:?}"),
    }
}

#[tokio::test]
async fn command_trigger_stdout_overflow_does_not_block_child() {
    // Emit 128 KiB of stdout (past Linux's default 64 KiB pipe buffer)
    // AND exit cleanly. Without the concurrent drain_to_ring task the
    // child would block waiting for the parent to read stdout, and
    // child.wait() would hang. The test must complete within the test
    // framework's default timeout (typically 60s on CI).
    //
    // `head -c 131072 /dev/zero` is the most portable way to emit a
    // fixed byte count to stdout: head's -c flag is POSIX 2008,
    // /dev/zero is on every Unix the trigger supports, and the
    // content (NUL bytes) does not matter for this test (the goal
    // is to overflow the pipe buffer, not assert byte values).
    let cfg = build_command_config("head -c 131072 /dev/zero");
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
}

#[tokio::test]
async fn command_trigger_aviso_env_vars_injected_into_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outfile = dir.path().join("out.txt");
    let cfg = build_command_config(format!(
        "printf '%s|%s|%s' \"$AVISO_EVENT_TYPE\" \"$AVISO_SEQUENCE\" \"$AVISO_IDENTIFIER_COUNTRY\" > '{}'",
        outfile.display()
    ));
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
    let contents = std::fs::read_to_string(&outfile).expect("read outfile");
    assert_eq!(contents, "mars|42|uk");
}

#[tokio::test]
async fn command_trigger_user_env_overrides_aviso_injection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outfile = dir.path().join("out.txt");
    let mut cfg = build_command_config(format!(
        "printf '%s' \"$AVISO_EVENT_TYPE\" > '{}'",
        outfile.display()
    ));
    cfg.env
        .insert("AVISO_EVENT_TYPE".to_string(), "OVERRIDDEN".to_string());
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
    let contents = std::fs::read_to_string(&outfile).expect("read outfile");
    assert_eq!(contents, "OVERRIDDEN");
}

#[tokio::test]
async fn command_trigger_working_dir_missing_returns_io_error() {
    let mut cfg = build_command_config("true");
    cfg.working_dir = Some(std::path::PathBuf::from(
        "/no/such/path/aviso-test-missing-dir-NEVER-CREATED-9d3b",
    ));
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    match result {
        Err(TriggerError::Io(_)) => {}
        other => panic!("expected Io error for missing working_dir, got {other:?}"),
    }
}

#[tokio::test]
async fn command_trigger_template_substitution_renders_notification_field() {
    let dir = tempfile::tempdir().expect("tempdir");
    let outfile = dir.path().join("out.txt");
    let cfg = build_command_config(format!(
        "printf 'event:{{{{ notification.event_type }}}}' > '{}'",
        outfile.display()
    ));
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    assert!(matches!(result, Ok(())), "got: {result:?}");
    let contents = std::fs::read_to_string(&outfile).expect("read outfile");
    assert_eq!(contents, "event:mars");
}

#[tokio::test]
async fn command_trigger_template_compile_failure_surfaces_at_dispatch() {
    // Builder is infallible: a malformed template produces a
    // CommandConfig whose command_template stores Err(...).
    let cfg = build_command_config("hello {{ notification.event_type");
    let result = dispatch_command(&cfg, None, &make_notification()).await;
    match result {
        Err(TriggerError::Template {
            context,
            field,
            kind,
        }) => {
            assert_eq!(context, "command");
            assert_eq!(field, "unclosed_braces");
            assert_eq!(kind, crate::watch::TemplateErrorKind::BadSyntax);
        }
        other => panic!("expected Template error, got {other:?}"),
    }
}

#[tokio::test]
async fn normalize_key_uppercases_and_replaces_non_alphanumerics() {
    assert_eq!(normalize_key("country-code"), "COUNTRY_CODE");
    assert_eq!(normalize_key("a.b.c"), "A_B_C");
    assert_eq!(normalize_key("X1y2"), "X1Y2");
    assert_eq!(normalize_key(""), "");
}

#[tokio::test]
async fn drain_to_ring_keeps_tail_when_input_exceeds_cap() {
    let cursor = std::io::Cursor::new(b"abcdefghij".to_vec());
    let tail = drain_to_ring(cursor, 4, "test").await;
    assert_eq!(&tail, b"ghij");
}

#[tokio::test]
async fn drain_to_ring_returns_full_input_when_under_cap() {
    let cursor = std::io::Cursor::new(b"abc".to_vec());
    let tail = drain_to_ring(cursor, 4096, "test").await;
    assert_eq!(&tail, b"abc");
}

#[tokio::test]
async fn drain_to_ring_handles_single_read_larger_than_cap() {
    // 4096 bytes fills the per-read 4096-byte buffer iter (the function
    // reads 4096 at a time). A single read of >= cap exercises the
    // special-case `if n >= cap` branch.
    let bytes: Vec<u8> = (0u32..4096)
        .map(|i| u8::try_from(i % 256).unwrap_or(0))
        .collect();
    let cursor = std::io::Cursor::new(bytes.clone());
    let tail = drain_to_ring(cursor, 100, "test").await;
    assert_eq!(tail.len(), 100);
    // The tail must equal the LAST 100 bytes of the input.
    assert_eq!(&tail[..], &bytes[bytes.len() - 100..]);
}
