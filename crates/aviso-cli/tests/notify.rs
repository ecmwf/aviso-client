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
