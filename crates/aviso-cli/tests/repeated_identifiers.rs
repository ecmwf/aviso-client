// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Repeated identifier arguments, including the HTTP boundary.

mod common;

use common::aviso;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn all_commands_send_exact_strings_and_typed_json() {
    let server = MockServer::start().await;
    let identifiers = json!({
        "class":"od", "label":" a,b\"'\\$HOME$(false)=x:=y ",
        "step":12, "fraction":1.25, "enabled":true, "missing":null,
        "array":[1,"two"], "severity":{"gte":5}, "string":"quoted"
    });
    for (command, endpoint) in [
        ("notify", "notification"),
        ("listen", "watch"),
        ("replay", "replay"),
    ] {
        // A terminal HTTP error avoids reconnects while still capturing the request.
        let mut expected = json!({"event_type":"mars", "identifier":identifiers});
        if command == "notify" {
            expected["payload"] = json!({"fixed":true});
        }
        Mock::given(method("POST"))
            .and(path(format!("/api/v1/{endpoint}")))
            .and(body_partial_json(expected))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let mut cli = aviso();
        cli.args(["--base-url", &server.uri(), command]);
        match command {
            "notify" => {
                cli.arg("event=mars,class=od,data={\"fixed\":true}");
            }
            "listen" => {
                cli.args([
                    "--event",
                    "mars",
                    "--no-state-store",
                    "/nonexistent/ignored.yaml",
                ]);
            }
            _ => {
                cli.args([
                    "--event",
                    "mars",
                    "--from",
                    "0",
                    "--listener",
                    "ignored",
                    "/nonexistent/ignored.yaml",
                ]);
            }
        }
        for entry in [
            "class=od",
            "label= a,b\"'\\$HOME$(false)=x:=y ",
            "step:=12",
            "fraction:=1.25",
            "enabled:=true",
            "missing:=null",
            "array:=[1,\"two\"]",
            "severity:={\"gte\":5}",
            "string:=\"quoted\"",
        ] {
            if command == "notify" && entry == "class=od" {
                continue;
            }
            cli.args(["--identifier", entry]);
        }
        cli.timeout(std::time::Duration::from_secs(10))
            .assert()
            .code(1);
    }
    server.verify().await;
}

#[tokio::test]
async fn usage_errors_happen_before_http() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    let cases = [
        vec!["notify", "event=mars", "--identifier", "event=mars"],
        vec!["notify", "event=mars", "--identifier", "data:={}"],
        vec!["notify", "event=mars,class=od", "--identifier", "class:=1"],
        vec!["notify", "--identifier", "class=od"],
        vec!["listen", "--identifier", "class=od"],
        vec!["listen", "--event", "mars"],
        vec![
            "listen",
            "--event",
            "mars",
            "--identifiers",
            "{}",
            "--identifier",
            "class=od",
        ],
        vec!["replay", "--from", "0", "--identifier", "class=od"],
        vec!["replay", "--from", "0", "--event", "mars"],
        vec![
            "replay",
            "--from",
            "0",
            "--event",
            "mars",
            "--identifiers",
            "{}",
            "--identifier",
            "class=od",
        ],
    ];
    for args in cases {
        aviso()
            .args(["--base-url", &server.uri()])
            .args(args)
            .assert()
            .code(2);
    }
    for command in ["notify", "listen", "replay"] {
        for entries in [
            vec!["missing"],
            vec!["=value"],
            vec!["x:=bad"],
            vec!["x=1", "x:=2"],
        ] {
            let mut cli = aviso();
            cli.args(["--base-url", &server.uri(), command]);
            if command == "notify" {
                cli.arg("event=mars");
            } else {
                cli.args(["--event", "mars"]);
            }
            if command == "replay" {
                cli.args(["--from", "0"]);
            }
            for entry in entries {
                cli.args(["--identifier", entry]);
            }
            cli.assert().code(2);
        }
    }
    assert!(
        server
            .received_requests()
            .await
            .ok_or_else(|| anyhow::anyhow!("request recording disabled"))?
            .is_empty()
    );
    Ok(())
}
