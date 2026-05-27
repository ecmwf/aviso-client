//! E2E: `aviso listen <yaml>` dispatches an echo trigger for matching publishes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic on unexpected variant is the standard test diagnostic"
)]

use std::io::Write;
use std::process::{Command as StdCommand, Stdio};
use std::thread;
use std::time::Duration;

use aviso_e2e::{PRODUCER_PASSWORD, PRODUCER_USERNAME, base_url};
use tempfile::NamedTempFile;

const POLYGON: &str = "0,40,1,40,1,41,0,40";

#[ignore = "requires e2e compose stack; run via stack_up.sh"]
#[test]
fn cli_listen_yaml_dispatches_echo_trigger() {
    let listener_yaml = format!(
        "listeners:\n  - event: test_polygon\n    identifiers:\n      polygon: \"{POLYGON}\"\n    triggers:\n      - type: echo\n"
    );
    let mut listener_file = NamedTempFile::new().unwrap();
    listener_file.write_all(listener_yaml.as_bytes()).unwrap();
    let listener_path = listener_file.path().to_path_buf();

    let aviso_bin = assert_cmd::cargo::cargo_bin("aviso");
    let base = base_url();

    let mut child = StdCommand::new(&aviso_bin)
        .args([
            "--base-url",
            &base,
            "--username",
            PRODUCER_USERNAME,
            "--password",
            PRODUCER_PASSWORD,
            "listen",
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
        let publish = StdCommand::new(&aviso_bin)
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
            .output()
            .unwrap();
        assert!(
            publish.status.success(),
            "publish seq={seq} should succeed; status={:?} stdout={:?} stderr={:?}",
            publish.status.code(),
            String::from_utf8_lossy(&publish.stdout),
            String::from_utf8_lossy(&publish.stderr),
        );
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
