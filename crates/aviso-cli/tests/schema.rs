// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for `aviso schema list` and `aviso schema get`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use predicates::str::contains;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::aviso;

fn catalogue_body() -> serde_json::Value {
    serde_json::json!({
        "status": "success",
        "schema": {
            "mars": {
                "payload": { "type": "object" },
                "identifier": {
                    "class": { "rule": "any" }
                }
            },
            "polygon": {
                "identifier": {
                    "region": { "rule": "any" }
                }
            }
        },
        "event_types": ["mars", "polygon"],
        "total_schemas": 2
    })
}

fn single_body() -> serde_json::Value {
    serde_json::json!({
        "status": "success",
        "event_type": "mars",
        "schema": {
            "payload": { "type": "object" },
            "identifier": {
                "class": { "rule": "any" }
            }
        }
    })
}

#[tokio::test]
async fn schema_list_returns_event_types() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(200).set_body_json(catalogue_body()))
        .mount(&server)
        .await;

    aviso()
        .args(["--base-url", &server.uri(), "schema", "list"])
        .assert()
        .success()
        .stdout(contains("mars"))
        .stdout(contains("polygon"));
}

#[tokio::test]
async fn schema_list_json_form_emits_ndjson() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(200).set_body_json(catalogue_body()))
        .mount(&server)
        .await;

    aviso()
        .args(["--base-url", &server.uri(), "--json", "schema", "list"])
        .assert()
        .success()
        .stdout(contains("\"event_type\":\"mars\""))
        .stdout(contains("\"event_type\":\"polygon\""));
}

#[tokio::test]
async fn schema_get_returns_pretty_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema/mars"))
        .respond_with(ResponseTemplate::new(200).set_body_json(single_body()))
        .mount(&server)
        .await;

    aviso()
        .args(["--base-url", &server.uri(), "schema", "get", "mars"])
        .assert()
        .success()
        .stdout(contains("\"event_type\": \"mars\""))
        .stdout(contains("\"identifier\""));
}

#[tokio::test]
async fn schema_get_missing_returns_exit_1() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema/bogus"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not configured"))
        .mount(&server)
        .await;

    aviso()
        .args(["--base-url", &server.uri(), "schema", "get", "bogus"])
        .assert()
        .failure()
        .code(1);
}

#[tokio::test]
async fn schema_list_without_base_url_exits_2() {
    aviso()
        .args(["schema", "list"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("base_url"));
}

#[tokio::test]
async fn schema_list_auth_401_exits_1() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    aviso()
        .args(["--base-url", &server.uri(), "schema", "list"])
        .assert()
        .failure()
        .code(1);
}
