//! E2E: `aviso notify` against the local stack with producer credentials.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic on unexpected variant is the standard test diagnostic"
)]

use std::time::Duration;

use assert_cmd::Command;
use aviso_e2e::{PRODUCER_PASSWORD, PRODUCER_USERNAME, base_url, isolated_aviso_command};

const POLYGON: &str = "0,30,1,30,1,31,0,30";

#[ignore = "requires e2e compose stack; run via stack.sh up"]
#[test]
fn cli_publish_succeeds_with_producer_credentials() {
    let parameters = format!(
        "event=test_polygon,polygon=\"{POLYGON}\",date=20260612,time=0000,data={{\"src\":\"cli_test\"}}"
    );
    Command::from_std(isolated_aviso_command())
        .args([
            "--base-url",
            &base_url(),
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
}
