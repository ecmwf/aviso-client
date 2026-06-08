// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for global flag effects on the CLI session.
//!
//! Covers the -v / -vv verbosity, --json forcing, and the
//! --danger-accept-invalid-certs session-level WARN emission.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect/panic on assert_cmd assertions and JSON-shape assertions is the expected diagnostic"
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
async fn otel_log_format_emits_required_fields_per_data_model() {
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
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();

    let line = stderr
        .lines()
        .find(|l| l.contains("\"event.name\":\"cli.config.resolved\""))
        .unwrap_or_else(|| {
            panic!("expected cli.config.resolved DEBUG event in stderr; got: {stderr}")
        });
    let parsed: serde_json::Value = serde_json::from_str(line)
        .unwrap_or_else(|e| panic!("emitted event must be valid JSON: {e}; line: {line}"));

    assert!(
        parsed
            .get("timestamp")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|t| t.ends_with('Z')),
        "expected RFC3339 UTC timestamp ending in Z; got: {parsed}"
    );
    assert_eq!(
        parsed
            .get("severityText")
            .and_then(serde_json::Value::as_str),
        Some("DEBUG"),
        "expected severityText=DEBUG; got: {parsed}"
    );
    assert_eq!(
        parsed
            .get("severityNumber")
            .and_then(serde_json::Value::as_u64),
        Some(5),
        "expected severityNumber=5 (OTel lowest-in-range for DEBUG); got: {parsed}"
    );
    assert!(
        parsed
            .get("body")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|b| !b.is_empty()),
        "expected non-empty body; got: {parsed}"
    );

    let resource = parsed.get("resource").expect("missing resource block");
    assert_eq!(
        resource
            .get("service.name")
            .and_then(serde_json::Value::as_str),
        Some("aviso-cli"),
        "resource.service.name must be aviso-cli; got: {resource}"
    );
    assert!(
        resource
            .get("service.version")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|v| !v.is_empty()),
        "resource.service.version must be present and non-empty; got: {resource}"
    );

    let attributes = parsed.get("attributes").expect("missing attributes block");
    assert_eq!(
        attributes
            .get("event.name")
            .and_then(serde_json::Value::as_str),
        Some("cli.config.resolved"),
        "attributes.event.name should carry the stable event identifier; got: {attributes}"
    );

    assert!(
        parsed.get("level").is_none(),
        "old tracing-subscriber `level` field should not appear in OTel-shape output; got: {parsed}"
    );
    assert!(
        parsed.get("fields").is_none(),
        "old tracing-subscriber `fields` block should not appear in OTel-shape output; got: {parsed}"
    );
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
