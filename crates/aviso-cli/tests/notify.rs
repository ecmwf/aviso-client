// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for `aviso notify`.
//!
//! Uses wiremock to fake the aviso-server's POST /api/v1/notification
//! endpoint. Each test sets `--base-url` to the mock server's URI
//! and asserts on stdout / stderr / exit code.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use predicates::prelude::*;
use predicates::str::contains;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::aviso;

#[tokio::test]
async fn numeric_range_http_errors_have_contextual_hints() {
    // Server responses for FloatHandler range [-100, 100] and IntHandler
    // range [0, 100000]. Float errors can display whole numbers without a dot.
    for (command, endpoint, title) in [
        ("notify", "notification", "Notification"),
        ("listen", "watch", "Watch"),
        ("replay", "replay", "Replay"),
    ] {
        for (field, value, range) in [
            ("anomaly", serde_json::json!(101), "[-100, 100]"),
            ("anomaly", serde_json::json!(100.5), "[-100, 100]"),
            ("step", serde_json::json!(100_001), "[0, 100000]"),
        ] {
            let server = MockServer::start().await;
            let qualifier = if command == "notify" {
                ""
            } else {
                "constraint "
            };
            let details = format!(
                "Field '{field}' {qualifier}value {value} is outside allowed range {range}"
            );
            let identifier = if command == "notify" {
                let mut identifiers = serde_json::json!({"anomaly": "0", "step": "0"});
                identifiers[field] = serde_json::json!(value.to_string());
                identifiers
            } else {
                serde_json::json!({field: {"gte": value}})
            };
            Mock::given(method("POST"))
                .and(path(format!("/api/v1/{endpoint}")))
                .and(body_partial_json(serde_json::json!({
                    "event_type": "weather", "identifier": identifier
                })))
                .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                    "code": format!("INVALID_{}_REQUEST", title.to_uppercase()),
                    "details": details,
                    "error": format!("Invalid {title} Request"),
                    "message": details,
                    "request_id": "range-error"
                })))
                .expect(1)
                .mount(&server)
                .await;

            let mut cli = aviso();
            cli.args(["--base-url", &server.uri(), command]);
            if command == "notify" {
                cli.arg(format!(
                    "event=weather,anomaly={},step={}",
                    identifier["anomaly"], identifier["step"]
                ));
            } else {
                cli.args([
                    "--event",
                    "weather",
                    "--identifiers",
                    &identifier.to_string(),
                ]);
                if command == "replay" {
                    cli.args(["--from", "0"]);
                } else {
                    cli.arg("--no-state-store");
                }
            }
            let action = if command == "listen" {
                "Check the identifier value in your repeated `--identifier` arguments or `--identifiers` JSON (inline mode), or in the listener YAML's `identifiers:` block (YAML mode)"
            } else {
                "Check the value you supplied for this identifier"
            };
            let hint = format!(
                "numeric identifier value is outside the schema's allowed range (the server lists [min, max] inline above). {action}; run `aviso schema get <TYPE>` for the authoritative schema (handler type and constraints)."
            );
            cli.timeout(std::time::Duration::from_secs(10))
                .assert()
                .failure()
                .code(1)
                .stderr(contains("http 400"))
                .stderr(contains(&details))
                .stderr(contains(hint))
                .stderr(contains("integer identifier value").not());
        }
    }
}

fn notify_success_body() -> serde_json::Value {
    serde_json::json!({
        "status": "success",
        "request_id": "req-abc",
        "processed_at": "2026-05-17T12:34:56Z",
    })
}

async fn mount_notify_success(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/api/v1/notification"))
        .respond_with(ResponseTemplate::new(200).set_body_json(notify_success_body()))
        .mount(server)
        .await;
}

#[tokio::test]
async fn happy_path_with_event_and_identifiers() {
    let server = MockServer::start().await;
    mount_notify_success(&server).await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "--json",
            "notify",
            "event=mars,class=od,stream=oper",
        ])
        .assert()
        .success()
        .stdout(contains("\"event_type\":\"mars\""))
        .stdout(contains("\"status\":\"success\""))
        .stdout(contains("\"request_id\":\"req-abc\""));
}

#[tokio::test]
async fn embedded_json_object_payload_is_accepted() {
    let server = MockServer::start().await;
    mount_notify_success(&server).await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "--json",
            "notify",
            r#"event=mars,data={"a":1,"b":2}"#,
        ])
        .assert()
        .success()
        .stdout(contains("\"status\":\"success\""));
}

#[tokio::test]
async fn point_cloud_identifier_reaches_http_body_as_array() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/notification"))
        .and(body_partial_json(serde_json::json!({
            "identifier": {
                "point_cloud": [[46, 8], [47, 9]],
                "enabled": "true",
            },
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(notify_success_body()))
        .expect(1)
        .mount(&server)
        .await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "notify",
            "event=observations,point_cloud=[[46,8],[47,9]],enabled=true",
        ])
        .assert()
        .success();
}

#[tokio::test]
async fn explicit_json_scalars_reach_http_body_with_their_types() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/notification"))
        .and(body_partial_json(serde_json::json!({
            "identifier": {
                "count": 12,
                "enabled": true,
                "missing": null,
                "label": "archive",
                "point": [46, 8],
                "area": {"north": 47, "south": 46},
            },
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(notify_success_body()))
        .expect(1)
        .mount(&server)
        .await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "notify",
            r#"event=observations,count:=12,enabled:=true,missing:=null,label:="archive",point:=[46,8],area:={"north":47,"south":46}"#,
        ])
        .assert()
        .success();
}

#[test]
fn invalid_explicit_json_identifier_exits_2_with_context() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "notify",
            "event=mars,count:=twelve",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("count"))
        .stderr(contains(":=JSON"))
        .stderr(contains("invalid JSON"))
        .stderr(contains("line"))
        .stderr(contains("column"));
}

#[tokio::test]
async fn quoted_string_with_literal_comma_in_payload_is_accepted() {
    let server = MockServer::start().await;
    mount_notify_success(&server).await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "--json",
            "notify",
            r#"event=mars,data={"msg":"hello, world"}"#,
        ])
        .assert()
        .success();
}

#[test]
fn missing_event_key_exits_2() {
    aviso()
        .args(["--base-url", "http://unused", "notify", "class=od"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("event="));
}

#[test]
fn empty_data_value_exits_2_with_suggestion() {
    aviso()
        .args(["--base-url", "http://unused", "notify", "event=mars,data="])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("data="));
}

#[test]
fn unclosed_brace_exits_2() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "notify",
            "event=mars,data={bad",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("unclosed"));
}

#[test]
fn invalid_json_in_data_exits_2_with_line_column() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "notify",
            r#"event=mars,data={"a":}"#,
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("line"));
}

#[tokio::test]
async fn server_500_surfaces_as_exit_1() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/notification"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream failed"))
        .mount(&server)
        .await;

    aviso()
        .args(["--base-url", &server.uri(), "notify", "event=mars,class=od"])
        .assert()
        .failure()
        .code(1);
}

#[tokio::test]
async fn unset_base_url_exits_2() {
    aviso()
        .args(["notify", "event=mars,class=od"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("base_url"));
}

#[tokio::test]
async fn tty_form_emits_human_readable_line_via_pipe() {
    let server = MockServer::start().await;
    mount_notify_success(&server).await;

    aviso()
        .args(["--base-url", &server.uri(), "notify", "event=mars,class=od"])
        .assert()
        .success()
        .stdout(contains("notification accepted").or(contains("\"event_type\"")));
}
