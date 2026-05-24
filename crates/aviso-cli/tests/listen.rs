//! Integration tests for `aviso listen`.
//!
//! These tests exercise the CLI-unique surface: Amendment C
//! resolution rules (positional vs config), the no-listeners
//! error per Amendment C, the unset-base-url usage error, and
//! the YAML parse error path. The watch-mode reconnect loop is
//! a library concern (covered by lib integration tests); CLI
//! tests focus on what the CLI alone owns.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use std::io::Write as _;

use predicates::prelude::*;
use predicates::str::contains;
use tempfile::{NamedTempFile, tempdir};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::aviso;

async fn mount_listen_finite_stream(server: &MockServer) {
    // Listen reconnects on `end_of_stream` (per the supervisor's
    // watch-mode contract). To make the test terminate inside its
    // bounded timeout, mount the success stream as a one-shot
    // (`up_to_n_times(1)`) and add a catch-all 404 mock for the
    // reconnect attempt; the supervisor classifies 4xx as terminal
    // and the process exits deterministically with code 1. The
    // exit code is NOT proof the feature failed; the real assertions
    // live in the calling tests (banner naming `ad-hoc`, stdout
    // containing the notification JSON). SSE format matches the
    // lib's own test helpers (event name `live-notification`,
    // cloud-event JSON shape from
    // `crates/aviso/tests/watch_supervisor_triggers.rs`).
    let cloud_event = concat!(
        "{\"id\":\"mars@1\",\"source\":\"https://aviso.example\",",
        "\"type\":\"int.ecmwf.aviso.mars\",\"time\":\"2026-05-17T12:34:56Z\",",
        "\"data\":{\"identifier\":{\"class\":\"od\"},\"payload\":{\"note\":\"inline-listen-happy-path\"}}}",
    );
    let body = format!(
        "event: live-notification\ndata: {cloud_event}\n\nevent: connection-closing\ndata: {{\"reason\":\"end_of_stream\",\"timestamp\":\"2026-05-17T13:00:00Z\",\"message\":\"\",\"topic\":\"mars\",\"request_id\":\"req-eos\"}}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .up_to_n_times(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/watch"))
        .respond_with(ResponseTemplate::new(404).set_body_string("end"))
        .mount(server)
        .await;
}

fn write_listener_file(yaml_body: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("create tempfile");
    f.write_all(yaml_body.as_bytes()).expect("write yaml");
    f
}

#[test]
fn listen_with_no_positional_and_empty_config_exits_2_with_helpful_message() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "").unwrap();

    aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains("no listeners to run"))
        .stderr(contains("pass listener YAML files"));
}

#[test]
fn listen_yaml_parse_error_includes_listener_file_path() {
    let listener = write_listener_file("listeners:\n  - bogus_field: 1\n    event: mars\n");
    let listener_path = listener.path().to_path_buf();

    aviso()
        .args([
            "--base-url",
            "http://unused",
            "listen",
            listener_path.to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(1)
        .stderr(contains(listener_path.display().to_string()).or(contains("bogus_field")));
}

#[test]
fn listen_unset_base_url_exits_2() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "").unwrap();
    let listener = write_listener_file("listeners:\n  - event: mars\n");

    aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "listen",
            listener.path().to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains("base_url"));
}

#[test]
fn listen_no_listeners_message_names_config_path() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "").unwrap();

    aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains(cfg_path.display().to_string()));
}

#[test]
fn listen_no_listeners_with_no_positional_does_not_claim_section_absent() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "listeners: []\n").unwrap();

    let assertion = aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2);
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        !stderr.contains("no `listeners:` section"),
        "stderr must not falsely claim the section is absent when it is present-but-empty; got: {stderr}"
    );
    assert!(
        stderr.contains("resolved to 0 entries")
            || stderr.contains("absent, present-but-empty, or commented out"),
        "stderr should describe the empty-resolution accurately; got: {stderr}"
    );
}

#[test]
fn listen_no_listeners_with_relative_positional_path_renders_absolute_in_at_line() {
    let listener = write_listener_file("listeners: []\n");
    let listener_dir = listener.path().parent().expect("tempfile has a parent");
    let listener_filename = listener
        .path()
        .file_name()
        .expect("tempfile has a name")
        .to_str()
        .expect("tempfile name is utf-8");
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "").unwrap();

    let assertion = aviso()
        .current_dir(listener_dir)
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
            listener_filename,
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2);
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    let absolute_path = listener.path().display().to_string();
    assert!(
        stderr.contains(&absolute_path),
        "at: line should quote absolute path (per Error UX rule 3), not the relative form the operator typed; got stderr: {stderr}"
    );
}

#[test]
fn listen_no_listeners_with_positional_yaml_attributes_to_positional_path_not_config() {
    let listener = write_listener_file("listeners: []\n");
    let listener_path = listener.path().to_path_buf();
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "listeners:\n  - event: never-resolved\n").unwrap();

    let assertion = aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
            listener_path.to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2);
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains(listener_path.display().to_string().as_str()),
        "stderr should name the positional file path (the source of the empty resolution); got: {stderr}"
    );
    assert!(
        !stderr.contains(cfg_path.display().to_string().as_str()),
        "stderr must NOT name the config path when positional files were supplied (Amendment C: positional REPLACES global); got: {stderr}"
    );
    assert!(
        stderr.contains("ensure each positional listener file") || stderr.contains("non-empty"),
        "stderr should suggest fixing the positional file; got: {stderr}"
    );
}

#[test]
fn listen_inline_event_without_identifiers_rejected_by_clap_with_pair_hint() {
    aviso()
        .args(["--base-url", "http://unused", "listen", "--event", "mars"])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains("--identifiers"));
}

#[test]
fn listen_inline_identifiers_without_event_rejected_by_clap_with_pair_hint() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "listen",
            "--identifiers",
            "{}",
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains("--event"));
}

#[test]
fn listen_inline_invalid_identifiers_json_exits_2_with_canonical_example_in_hint() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "listen",
            "--event",
            "mars",
            "--identifiers",
            "{not valid json",
            "--no-state-store",
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains("parse --identifiers"))
        .stderr(contains(r#"'{"class":"od"}'"#));
}

#[tokio::test]
async fn listen_inline_overrides_positional_yaml_when_both_present_uses_ad_hoc_listener() {
    // Positional flag ordering: yaml file FIRST, inline flags AFTER.
    // This pins the clap variadic-positional edge case where a later
    // long-flag could otherwise be misparsed as a positional argument.
    let listener = write_listener_file("listeners:\n  - name: from-yaml\n    event: other\n");
    let server = MockServer::start().await;
    mount_listen_finite_stream(&server).await;

    let assertion = aviso()
        .args([
            "--base-url",
            &server.uri(),
            "listen",
            listener.path().to_str().unwrap(),
            "--event",
            "mars",
            "--identifiers",
            r#"{"class":"od"}"#,
            "--no-state-store",
        ])
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        // The mock returns a finite stream followed by 404 on reconnect
        // (see `mount_listen_finite_stream`), so exit code is 1 from the
        // 4xx-terminal classification. The test's real assertion is the
        // startup-banner content below, which only fires AFTER inline
        // resolution succeeded and the supervisor started.
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("ad-hoc"),
        "startup banner must name the ad-hoc inline listener (proving inline-overrides-yaml precedence took effect, not the yaml listener `from-yaml`); got stderr: {stderr}",
    );
    assert!(
        !stderr.contains("from-yaml"),
        "the YAML-defined listener `from-yaml` must NOT appear in the startup banner when inline flags are also present; inline takes precedence and yaml is silently ignored (matches aviso replay's existing behavior); got stderr: {stderr}",
    );
}

#[tokio::test]
async fn listen_inline_emits_notification_on_stdout_via_default_echo_trigger() {
    let server = MockServer::start().await;
    mount_listen_finite_stream(&server).await;

    let assertion = aviso()
        .args([
            "--base-url",
            &server.uri(),
            "listen",
            "--event",
            "mars",
            "--identifiers",
            r#"{"class":"od"}"#,
            "--no-state-store",
        ])
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        // Same termination path as the override test above: success
        // stream, then 404 on reconnect, exit code 1. The real assertion
        // is on stdout content (proving the default echo trigger fired).
        .failure()
        .code(1);
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).to_string();
    assert!(
        stdout.contains("\"event_type\":\"mars\""),
        "default echo trigger must emit the notification JSON on stdout when inline mode delivers an event; without this the inline-listen path silently discards events and looks broken to operators piping to jq. Got stdout: {stdout}",
    );
    assert!(
        stdout.contains("\"sequence\":1"),
        "echo trigger must include the sequence field so downstream consumers (jq, ndjson tools) can disambiguate events; got stdout: {stdout}",
    );
    assert!(
        stdout.contains("inline-listen-happy-path"),
        "the payload from the mock notification must reach stdout verbatim through the default echo trigger; got stdout: {stdout}",
    );
}
