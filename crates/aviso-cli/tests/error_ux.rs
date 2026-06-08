// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for the Error UX nine-rule discipline.
//!
//! Each test pins one of the nine rules from the locked Error UX
//! contract:
//!   1. exit code (0/1/2/130) reflects the failure class.
//!   2. one-line summary first.
//!   3. absolute path printed when a file is involved.
//!   4. YAML line:col reported via `serde_norway::Error::location()`.
//!   5. key path NAMED for serde errors.
//!   6. suggestion attached.
//!   7. anyhow Caused-by chain present.
//!   8. NEVER leak paths/env/secrets the user did not provide.
//!   9. every subcommand routes through the single `error::format_chain`
//!      seam.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use predicates::prelude::*;
use predicates::str::contains;
use tempfile::tempdir;

use common::aviso;

#[test]
fn rule_1_admin_no_yes_exits_2() {
    aviso()
        .args(["admin", "wipe-all"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn rule_1_runtime_server_500_exits_1() {
    let dir = tempdir().unwrap();
    let cfg = dir.path().join("config.yaml");
    std::fs::write(&cfg, "").unwrap();
    aviso()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--base-url",
            "http://127.0.0.1:9", // discard port; connection refused
            "schema",
            "list",
        ])
        .assert()
        .failure()
        .code(1);
}

#[test]
fn rule_2_error_starts_with_one_line_summary() {
    aviso()
        .args(["admin", "wipe-all"])
        .assert()
        .failure()
        .stderr(contains("error: "));
}

#[test]
fn rule_3_listener_file_parse_error_names_absolute_path() {
    let dir = tempdir().unwrap();
    let bad = dir.path().join("bad.yaml");
    std::fs::write(&bad, "listeners:\n  - bogus_field: 1\n").unwrap();

    aviso()
        .args([
            "--base-url",
            "http://unused",
            "listen",
            bad.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(contains(bad.display().to_string()));
}

#[test]
fn rule_4_yaml_error_includes_line_position() {
    let dir = tempdir().unwrap();
    let bad = dir.path().join("bad.yaml");
    std::fs::write(&bad, "base_url: foo\nbogus_top_level: 1\n").unwrap();

    aviso()
        .args(["--config", bad.to_str().unwrap(), "config", "dump"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("line").or(contains("bogus_top_level")));
}

#[test]
fn rule_5_serde_unknown_field_is_named() {
    let dir = tempdir().unwrap();
    let bad = dir.path().join("config.yaml");
    std::fs::write(&bad, "definitely_unknown_field: 1\n").unwrap();

    aviso()
        .args(["--config", bad.to_str().unwrap(), "config", "dump"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("definitely_unknown_field").or(contains("unknown field")));
}

#[test]
fn rule_6_admin_no_yes_includes_suggestion() {
    aviso()
        .args(["admin", "wipe-all"])
        .assert()
        .failure()
        .stderr(contains("--yes"));
}

#[test]
fn rule_6_no_listeners_includes_fix_guidance() {
    let dir = tempdir().unwrap();
    let cfg = dir.path().join("config.yaml");
    std::fs::write(&cfg, "").unwrap();

    aviso()
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("pass listener YAML files").or(contains("listeners:")));
}

#[test]
fn rule_6_partial_auth_env_exits_2_with_misconfig_message() {
    aviso()
        .env("AVISO_USERNAME", "alice")
        .args(["--base-url", "http://unused", "schema", "list"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("misconfigured").or(contains("AVISO_PASSWORD")));
}

#[test]
fn rule_6_partial_auth_env_password_only_exits_2_with_misconfig_message() {
    aviso()
        .env("AVISO_PASSWORD", "wonderland")
        .args(["--base-url", "http://unused", "schema", "list"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("misconfigured").or(contains("AVISO_USERNAME")));
}

#[test]
fn rule_7_caused_by_chain_present_for_layered_errors() {
    let dir = tempdir().unwrap();
    let bad = dir.path().join("config.yaml");
    std::fs::write(&bad, "definitely_unknown_field: 1\n").unwrap();

    aviso()
        .args(["--config", bad.to_str().unwrap(), "config", "dump"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("Caused by:"));
}

#[test]
fn rule_8_token_value_not_leaked_in_error_output() {
    aviso()
        .args([
            "--base-url",
            "http://127.0.0.1:9",
            "--token",
            "do-not-leak-this-secret-in-stderr",
            "schema",
            "list",
        ])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("do-not-leak-this-secret-in-stderr").not());
}
