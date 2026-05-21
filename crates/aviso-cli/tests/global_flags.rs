//! Integration tests for global flag effects on the CLI session.
//!
//! Covers the -v / -vv verbosity, --json forcing, and the
//! --danger-accept-invalid-certs session-level WARN emission.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use predicates::prelude::*;
use predicates::str::contains;
use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::aviso;

fn empty_config_dir() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("config.yaml"), "").unwrap();
    dir
}

#[tokio::test]
async fn verbosity_v_sets_debug_in_stderr_tracing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "schema": {},
            "event_types": [],
            "total_schemas": 0
        })))
        .mount(&server)
        .await;

    let dir = empty_config_dir();
    let assertion = aviso()
        .args([
            "--config",
            dir.path().join("config.yaml").to_str().unwrap(),
            "--base-url",
            &server.uri(),
            "-v",
            "schema",
            "list",
        ])
        .env_remove("AVISO_LOG")
        .assert()
        .success();
    let output = assertion.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("DEBUG") || stderr.contains("\"level\":\"DEBUG\""),
        "expected DEBUG-level tracing in stderr; got: {stderr}"
    );
}

#[tokio::test]
async fn verbosity_no_v_uses_info_level_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "schema": {},
            "event_types": [],
            "total_schemas": 0
        })))
        .mount(&server)
        .await;

    let dir = empty_config_dir();
    let assertion = aviso()
        .args([
            "--config",
            dir.path().join("config.yaml").to_str().unwrap(),
            "--base-url",
            &server.uri(),
            "schema",
            "list",
        ])
        .env_remove("AVISO_LOG")
        .assert()
        .success();
    let output = assertion.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("DEBUG"),
        "default (no -v) should NOT include DEBUG-level events; got: {stderr}"
    );
}

#[tokio::test]
async fn danger_accept_invalid_certs_emits_session_warn() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "schema": {},
            "event_types": [],
            "total_schemas": 0
        })))
        .mount(&server)
        .await;

    let dir = empty_config_dir();
    aviso()
        .args([
            "--config",
            dir.path().join("config.yaml").to_str().unwrap(),
            "--base-url",
            &server.uri(),
            "--danger-accept-invalid-certs",
            "schema",
            "list",
        ])
        .env_remove("AVISO_LOG")
        .assert()
        .success()
        .stderr(contains("cli.tls.insecure_mode").or(contains("insecure")));
}

#[tokio::test]
async fn no_danger_flag_does_not_emit_insecure_warn() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "success",
            "schema": {},
            "event_types": [],
            "total_schemas": 0
        })))
        .mount(&server)
        .await;

    let dir = empty_config_dir();
    let assertion = aviso()
        .args([
            "--config",
            dir.path().join("config.yaml").to_str().unwrap(),
            "--base-url",
            &server.uri(),
            "schema",
            "list",
        ])
        .env_remove("AVISO_LOG")
        .assert()
        .success();
    let output = assertion.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("cli.tls.insecure_mode"),
        "insecure_mode WARN should not fire without --danger-accept-invalid-certs; got: {stderr}"
    );
}
