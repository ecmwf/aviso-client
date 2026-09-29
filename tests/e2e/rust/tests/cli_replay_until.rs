// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! E2E: `aviso replay --until` stops at the end point against the local stack.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use assert_cmd::Command;
use aviso::NotificationRequest;
use aviso_e2e::{
    PRODUCER_PASSWORD, PRODUCER_USERNAME, base_url, isolated_aviso_command, producer_client,
};
use serde_json::{Value, json};

const EVENT_TYPE: &str = "test_event";

/// Runs `aviso replay` with `window` and returns the sequences it printed.
fn replay(identifiers: &str, window: &[&str]) -> Vec<u64> {
    let base = base_url();
    let mut args = vec![
        "--base-url",
        &base,
        "--username",
        PRODUCER_USERNAME,
        "--password",
        PRODUCER_PASSWORD,
        "replay",
        "--event",
        EVENT_TYPE,
        "--identifiers",
        identifiers,
    ];
    args.extend_from_slice(window);
    let output = Command::from_std(isolated_aviso_command())
        .args(args)
        .timeout(Duration::from_secs(60))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["sequence"]
                .as_u64()
                .unwrap()
        })
        .collect()
}

#[ignore = "requires e2e compose stack; run via stack.sh up"]
#[tokio::test]
async fn cli_replay_until_stops_at_the_end_point() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let date = format!(
        "{}{:02}{:02}",
        2100 + nanos % 7900,
        1 + (nanos / 7900) % 12,
        1 + (nanos / 94_800) % 28
    );
    let identifiers = format!("{{\"date\":\"{date}\"}}");
    // Notifications a previous run left for the same date are excluded by
    // starting every replay after the last of them.
    let baseline = replay(&identifiers, &["--from", "0"])
        .into_iter()
        .max()
        .unwrap_or(0)
        .to_string();
    let client = producer_client().unwrap();
    for time in ["0000", "0600", "1200"] {
        let identifier = BTreeMap::from([
            ("date".to_string(), json!(date)),
            ("time".to_string(), json!(time)),
        ]);
        client
            .notify(&NotificationRequest::new(EVENT_TYPE).with_identifier(identifier))
            .await
            .unwrap();
    }

    let all = replay(&identifiers, &["--from", &baseline]);
    assert_eq!(all.len(), 3, "{all:?}");

    // The start is exclusive and the end inclusive.
    let from = all[0].to_string();
    let until = all[1].to_string();
    assert_eq!(
        replay(&identifiers, &["--from", &from, "--until", &until]),
        [all[1]]
    );
    // An end point past everything stored ends at the last notification.
    assert_eq!(
        replay(
            &identifiers,
            &["--from", &baseline, "--until", "2100-01-01"]
        ),
        all
    );
}
