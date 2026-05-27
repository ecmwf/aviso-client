//! E2E: `aviso replay --from <future-seq>` exits 0 with empty replay against the local stack.
//!
//! Replays from sequence `u64::MAX - 1` so the server's reply is empty (no historical items at
//! or after that cursor) and the test stays insensitive to the stream's accumulated state
//! across other tests in the session.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: panic on unexpected variant is the standard test diagnostic"
)]

use std::time::Duration;

use assert_cmd::Command;
use aviso_e2e::{PRODUCER_PASSWORD, PRODUCER_USERNAME, base_url, isolated_aviso_command};
use predicates::str::is_empty;

const POLYGON: &str = "0,50,1,50,1,51,0,50";

#[ignore = "requires e2e compose stack; run via stack_up.sh"]
#[test]
fn cli_replay_empty_window_exits_zero() {
    let base = base_url();
    let identifiers = format!("{{\"polygon\":\"{POLYGON}\"}}");

    Command::from_std(isolated_aviso_command())
        .args([
            "--base-url",
            &base,
            "--username",
            PRODUCER_USERNAME,
            "--password",
            PRODUCER_PASSWORD,
            "replay",
            "--from",
            &(u64::MAX - 1).to_string(),
            "--event",
            "test_polygon",
            "--identifiers",
            &identifiers,
        ])
        .timeout(Duration::from_secs(15))
        .assert()
        .success()
        .stdout(is_empty());
}
