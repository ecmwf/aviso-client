// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! CLI startup status, endpoint validation, and interrupt regressions.

mod common;

use std::time::Duration;

use predicates::prelude::*;
use predicates::str::contains;
use tokio::io::AsyncReadExt;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn unrecognized_json_errors_are_omitted_at_debug_verbosity() {
    for body in [
        serde_json::json!({"proxy_debug": "http://user:URL_SECRET@upstream/private?token=QUERY_SECRET"}),
        serde_json::json!({"code": "PROXY_FAILURE", "message": "PLAIN_SECRET"}),
        serde_json::json!({"code": "INVALID_WATCH_REQUEST", "details": {"debug": "PLAIN_SECRET"}}),
        serde_json::json!({"code": "INVALID_WATCH_REQUEST", "proxy_debug": "PLAIN_SECRET"}),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(body)
                    .insert_header(
                        "x-request-id",
                        "http://user:HEADER_SECRET@host/private?token=QUERY_SECRET",
                    ),
            )
            .expect(1)
            .mount(&server)
            .await;
        listen(&server.uri())
            .env_remove("AVISO_LOG")
            .arg("-v")
            .assert()
            .code(1)
            .stdout("")
            .stderr(contains("http 403"))
            .stderr(contains("unrecognized response body omitted"))
            .stderr(contains("cli.listener.failed"))
            .stderr(contains("SECRET").not())
            .stderr(contains("proxy_debug").not());
    }
}

#[tokio::test]
async fn aviso_shaped_errors_redact_urls_in_all_retained_fields() {
    for request_id_in_body in [false, true] {
        let server = MockServer::start().await;
        let url = "HtTp://user:URL_SECRET@upstream/private?token=QUERY_SECRET#FRAGMENT_SECRET";
        let mut body = serde_json::json!({
            "code": "UNKNOWN_EVENT_TYPE",
            "message": format!("unknown event type '{url}'"),
            "details": format!("Field 'step' constraint value 101 is outside allowed range [0, 100]. Check {url}"),
            "configured_event_types": ["mars", url],
            "error": "UNSELECTED_SECRET",
            "proxy_debug": "UNSELECTED_SECRET",
        });
        if request_id_in_body {
            body["request_id"] = serde_json::json!(url);
        }
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(body)
                    .insert_header("x-request-id", url),
            )
            .expect(1)
            .mount(&server)
            .await;
        listen(&server.uri())
            .env_remove("AVISO_LOG")
            .arg("-v")
            .assert()
            .code(1)
            .stdout("")
            .stderr(contains("UNKNOWN_EVENT_TYPE"))
            .stderr(contains("outside allowed range [0, 100]"))
            .stderr(contains("configured_event_types"))
            .stderr(contains("mars"))
            .stderr(contains("request_id"))
            .stderr(contains("[URL omitted]"))
            .stderr(contains("Hint:"))
            .stderr(contains("aviso schema list"))
            .stderr(contains("cli.listener.failed"))
            .stderr(contains("SECRET").not())
            .stderr(contains("upstream/private").not())
            .stderr(contains("proxy_debug").not());
    }
}

#[tokio::test]
async fn recognized_validation_details_and_hints_are_preserved() {
    let server = MockServer::start().await;
    let details = "Field 'anomaly' constraint value 101 is outside allowed range [-100, 100]";
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "code": "INVALID_WATCH_REQUEST", "error": "Invalid Watch Request",
            "message": details, "details": details, "request_id": "request-1",
            "debug": {"token": "UNSELECTED_SECRET"},
        })))
        .expect(1)
        .mount(&server)
        .await;
    listen(&server.uri())
        .assert()
        .code(1)
        .stdout("")
        .stderr(contains("http 400"))
        .stderr(contains("INVALID_WATCH_REQUEST"))
        .stderr(contains(details))
        .stderr(contains("request-1"))
        .stderr(contains("Hint: numeric identifier value"))
        .stderr(contains("SECRET").not());
}

fn listen(url: &str) -> assert_cmd::Command {
    let mut command = common::aviso();
    command
        .env("AVISO_BASE_URL", url)
        .args([
            "listen",
            "--event",
            "mars",
            "--identifiers",
            "{}",
            "--no-state-store",
        ])
        .timeout(Duration::from_secs(5));
    command
}

#[tokio::test]
async fn wrong_endpoint_reports_connecting_without_listening_or_secrets() {
    for status in [200, 401, 403, 404] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_raw("BODY_SECRET", "text/html"))
            .expect(1)
            .mount(&server)
            .await;
        let url = server.uri().replace("http://", "http://user:URL_SECRET@");
        listen(&format!("{url}/private?token=QUERY_SECRET"))
            .assert()
            .code(1)
            .stdout("")
            .stderr(contains("Connecting for"))
            .stderr(contains("Listening for").not())
            .stderr(contains("SECRET").not());
    }
}

#[tokio::test]
async fn retries_are_visible_named_and_bounded() {
    for status in [429, 500] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_string("BODY_SECRET"))
            .mount(&server)
            .await;
        listen(&server.uri())
            .env_remove("AVISO_LOG")
            .args(["--startup-timeout", "200ms"])
            .assert()
            .code(1)
            .stdout("")
            .stderr(contains("Retrying listener connection"))
            .stderr(contains("ad-hoc"))
            .stderr(contains("delay_ms"))
            .stderr(contains("startup timeout"))
            .stderr(contains("Listening for").not())
            .stderr(contains("SECRET").not());
    }
}

#[test]
fn startup_timeout_help_and_usage_errors() {
    common::aviso()
        .args(["listen", "--help"])
        .assert()
        .success()
        .stdout(contains("--startup-timeout <DURATION>"))
        .stdout(contains("30s"))
        .stdout(contains("0s to disable"));
    for value in ["-1s", "forever", "invalid"] {
        listen("http://127.0.0.1:1")
            .args(["--startup-timeout", value])
            .assert()
            .code(2)
            .stderr(contains("startup-timeout"));
    }
}

#[tokio::test]
async fn transient_failure_then_opening_reports_ready_once_and_json_stdout() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let body = concat!(
        "event: live-notification\ndata: {\"type\":\"connection_established\"}\n\n",
        "event: live-notification\ndata: {\"id\":\"mars@1\",\"data\":{\"identifier\":{}}}\n\n",
        "event: connection-closing\ndata: {\"reason\":\"max_duration_reached\"}\n\n",
    );
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(body, "Text/Event-Stream; charset=utf-8"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let output = listen(&server.uri())
        .args(["--startup-timeout", "0s"])
        .env_remove("AVISO_LOG")
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stderr.matches("Listening for").count(), 1);
    assert!(stderr.contains("Retrying listener connection"));
    let connecting = stderr
        .find("Connecting for")
        .ok_or("missing Connecting status")?;
    let listening = stderr
        .find("Listening for")
        .ok_or("missing Listening status")?;
    assert!(connecting < listening);
    let notification: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(notification["sequence"], 1);
    Ok(())
}

#[tokio::test]
async fn interrupt_while_waiting_for_headers_exits_cleanly() -> TestResult {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let child = tokio::process::Command::new(assert_cmd::cargo::cargo_bin!("aviso"))
        .env_clear()
        .env("AVISO_BASE_URL", url)
        .env(
            "AVISO_CLIENT_CONFIG_FILE",
            "/nonexistent/aviso-startup-test.yaml",
        )
        .args([
            "listen",
            "--event",
            "mars",
            "--identifiers",
            "{}",
            "--no-state-store",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await??;
    let mut request = [0; 4096];
    assert!(socket.read(&mut request).await? > 0);
    let pid = child.id().ok_or("CLI exited before interrupt")?;
    let signal = tokio::process::Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .await?;
    assert!(signal.success());
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output()).await??;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("Connecting for"));
    assert!(!stderr.contains("Listening for"));
    assert!(stderr.contains("All listeners stopped"));
    Ok(())
}
