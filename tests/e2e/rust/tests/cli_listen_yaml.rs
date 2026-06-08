// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! E2E: `aviso listen <yaml>` dispatches an echo trigger for matching publishes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic on unexpected variant is the standard test diagnostic"
)]

use std::io::Write;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use assert_cmd::Command as AssertCommand;
use aviso_e2e::{PRODUCER_PASSWORD, PRODUCER_USERNAME, base_url, isolated_aviso_command};
use tempfile::NamedTempFile;

const POLYGON: &str = "0,40,1,40,1,41,0,40";

#[ignore = "requires e2e compose stack; run via stack.sh up"]
#[test]
fn cli_listen_yaml_dispatches_echo_trigger() {
    let listener_yaml = format!(
        "listeners:\n  - event: test_polygon\n    identifiers:\n      polygon: \"{POLYGON}\"\n    triggers:\n      - type: echo\n"
    );
    let mut listener_file = NamedTempFile::new().unwrap();
    listener_file.write_all(listener_yaml.as_bytes()).unwrap();
    let listener_path = listener_file.path().to_path_buf();

    let base = base_url();

    let mut child = isolated_aviso_command()
        .args([
            "--base-url",
            &base,
            "--username",
            PRODUCER_USERNAME,
            "--password",
            PRODUCER_PASSWORD,
            "listen",
            "--no-state-store",
            listener_path.to_str().unwrap(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    thread::sleep(Duration::from_secs(2));

    for seq in 0..3 {
        let parameters = format!(
            "event=test_polygon,polygon=\"{POLYGON}\",date=20260613,time={seq:04},data={{\"from\":\"cli_listen_test\",\"seq\":{seq}}}"
        );
        AssertCommand::from_std(isolated_aviso_command())
            .args([
                "--base-url",
                &base,
                "--username",
                PRODUCER_USERNAME,
                "--password",
                PRODUCER_PASSWORD,
                "notify",
                &parameters,
            ])
            .timeout(Duration::from_secs(10))
            .assert()
            .success();
        thread::sleep(Duration::from_millis(300));
    }

    thread::sleep(Duration::from_secs(3));

    let _ = child.kill();
    let output = child.wait_with_output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("test_polygon") && stdout.contains(POLYGON),
        "echo trigger should emit NDJSON matching the published notification; \
         got stdout: {stdout:?}, stderr: {stderr:?}",
    );
}
