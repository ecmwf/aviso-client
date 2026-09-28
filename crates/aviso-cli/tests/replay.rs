// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for `aviso replay`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use std::io::Write as _;

use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use tempfile::NamedTempFile;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::aviso;

fn write_listener_file(yaml_body: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("create tempfile");
    f.write_all(yaml_body.as_bytes()).expect("write yaml");
    f
}

async fn mount_replay_finite_stream(server: &MockServer) {
    let body = concat!(
        "event: replay-control\ndata: {\"type\":\"replay_started\"}\n\n",
        "event: replay-control\n",
        "data: {\"type\":\"replay_completed\",\"timestamp\":\"2026-05-17T13:00:00Z\",\"topic\":\"mars\",\"request_id\":\"req-rc\"}\n\n",
        "event: connection-closing\n",
        "data: {\"reason\":\"end_of_stream\",\"timestamp\":\"2026-05-17T13:00:00Z\",\"message\":\"\",\"topic\":\"mars\",\"request_id\":\"req-eos\"}\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/replay"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

#[test]
fn replay_requires_from() {
    aviso().args(["replay"]).assert().failure().code(2);
}

#[test]
fn replay_with_bad_from_value_exits_2() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "replay",
            "--from",
            "not-a-date-and-not-an-id-and-has-dashes",
            "--listener",
            "x",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("--from"));
}

#[tokio::test]
async fn replay_with_single_resolved_listener_succeeds() {
    let server = MockServer::start().await;
    mount_replay_finite_stream(&server).await;

    let listener = write_listener_file("listeners:\n  - name: only\n    event: mars\n");

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "replay",
            "--from",
            "42",
            listener.path().to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

#[tokio::test]
async fn replay_with_multiple_listeners_requires_disambiguation() {
    let listener = write_listener_file(
        "listeners:\n  - name: a\n    event: mars\n  - name: b\n    event: era5\n",
    );

    aviso()
        .args([
            "--base-url",
            "http://unused",
            "replay",
            "--from",
            "42",
            listener.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("--listener"));
}

#[tokio::test]
async fn replay_with_listener_name_picks_correct_one() {
    let server = MockServer::start().await;
    mount_replay_finite_stream(&server).await;

    let listener = write_listener_file(
        "listeners:\n  - name: a\n    event: mars\n  - name: b\n    event: era5\n",
    );

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "replay",
            "--listener",
            "a",
            "--from",
            "42",
            listener.path().to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

#[tokio::test]
async fn replay_ad_hoc_with_event_and_identifiers_succeeds() {
    let server = MockServer::start().await;
    mount_replay_finite_stream(&server).await;

    aviso()
        .args([
            "--base-url",
            &server.uri(),
            "replay",
            "--event",
            "mars",
            "--identifiers",
            "{\"class\":\"od\"}",
            "--from",
            "42",
        ])
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success();
}

#[test]
fn replay_event_without_identifiers_exits_2() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "replay",
            "--event",
            "mars",
            "--from",
            "42",
        ])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn replay_invalid_identifiers_json_exits_2() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "replay",
            "--event",
            "mars",
            "--identifiers",
            "not json",
            "--from",
            "42",
        ])
        .assert()
        .failure()
        .code(2);
}

#[tokio::test]
async fn replay_until_sends_the_end_point_and_names_it_in_the_banner() {
    let server = MockServer::start().await;
    let body = concat!(
        "event: replay-control\ndata: {\"type\":\"replay_started\",\"end_sequence\":20}\n\n",
        "event: replay-control\n",
        "data: {\"type\":\"replay_completed\",\"timestamp\":\"2026-05-17T13:00:00Z\",\"topic\":\"mars\"}\n\n",
        "event: connection-closing\n",
        "data: {\"reason\":\"end_of_stream\",\"timestamp\":\"2026-05-17T13:00:00Z\",\"message\":\"\",\"topic\":\"mars\",\"request_id\":\"r\"}\n\n",
    );
    // Only a request with the expected end point is answered.
    for (until, wire) in [
        ("20", serde_json::json!({"from_id": "11", "to_id": "20"})),
        (
            "2026-05-18",
            serde_json::json!({"from_id": "11", "to_date": "2026-05-18T00:00:00.000000Z"}),
        ),
    ] {
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/replay"))
            .and(body_partial_json(wire))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .mount(&server)
            .await;
        let banner = if until == "20" {
            "from sequence #10 until sequence #20."
        } else {
            "from sequence #10 until 2026-05-18."
        };
        aviso()
            .args([
                "--base-url",
                &server.uri(),
                "replay",
                "--event",
                "mars",
                "--identifier",
                "class=od",
                "--from",
                "10",
                "--until",
                until,
            ])
            .timeout(std::time::Duration::from_secs(60))
            .assert()
            .success()
            .stderr(contains(banner))
            .stderr(contains("Replay complete."));
    }
}

#[test]
fn replay_with_bad_until_value_exits_2() {
    aviso()
        .args([
            "--base-url",
            "http://unused",
            "replay",
            "--from",
            "10",
            "--until",
            "tomorrow",
            "--listener",
            "x",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("--until value `tomorrow`"));
}

#[test]
fn replay_with_an_until_not_after_from_is_a_usage_error() {
    aviso()
        .args([
            "--base-url",
            "http://127.0.0.1:1",
            "replay",
            "--event",
            "mars",
            "--identifier",
            "class=od",
            "--from",
            "10",
            "--until",
            "10",
        ])
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .failure()
        .code(2)
        .stderr(contains("--until 10 is not after --from 10"))
        .stderr(contains("Replaying").not());
}
