// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for `aviso admin {wipe-stream,wipe-all,delete}`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use predicates::prelude::*;
use predicates::str::contains;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::aviso;

#[tokio::test]
async fn wipe_stream_success_path() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/admin/wipe/stream"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "admin",
            "wipe-stream",
            "mars",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(contains("ok"))
        .stdout(contains("wipe_stream"));
}

#[tokio::test]
async fn wipe_all_success_path() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/admin/wipe/all"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    aviso()
        .args(["--base-url", &server.uri(), "admin", "wipe-all", "--yes"])
        .assert()
        .success()
        .stdout(contains("ok"))
        .stdout(contains("wipe_all"));
}

#[tokio::test]
async fn delete_success_path() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/admin/notification/mars@42"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "admin",
            "delete",
            "mars@42",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(contains("ok"))
        .stdout(contains("delete_notification"));
}

#[tokio::test]
async fn wipe_stream_401_includes_credentials_message() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/admin/wipe/stream"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "admin",
            "wipe-stream",
            "mars",
            "--yes",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("auth failed").or(contains("credentials")));
}

#[tokio::test]
async fn wipe_stream_403_includes_admin_role_message() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/admin/wipe/stream"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "admin",
            "wipe-stream",
            "mars",
            "--yes",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("admin role").or(contains("forbidden")));
}

#[tokio::test]
async fn wipe_all_json_output_form() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/v1/admin/wipe/all"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "--json",
            "admin",
            "wipe-all",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(contains("\"status\":\"ok\""))
        .stdout(contains("\"operation\":\"wipe_all\""));
}
