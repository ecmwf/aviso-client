// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for `aviso config dump`.
//!
//! Exercises the resolved-config layering (flag > env > file >
//! default), source attribution, --redact masking, and the
//! YAML / --json output forms.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use std::path::PathBuf;

use predicates::prelude::*;
use predicates::str::contains;
use tempfile::tempdir;

use common::aviso_with_config;

fn write_config(dir: &std::path::Path, body: &str) -> PathBuf {
    let path = dir.join("config.yaml");
    std::fs::write(&path, body).expect("write config fixture");
    path
}

#[test]
fn dump_shows_resolved_paths_with_source_attribution() {
    let dir = tempdir().unwrap();
    let cfg = write_config(dir.path(), "base_url: https://from-file.example\n");

    aviso_with_config(&cfg)
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("config_path"))
        .stdout(contains("state_file"))
        .stdout(contains("# from: file"));
}

#[test]
fn dump_shows_config_path_attributed_to_flag_when_overridden() {
    let dir = tempdir().unwrap();
    let cfg = write_config(dir.path(), "base_url: https://from-file.example\n");

    let assertion = aviso_with_config(&cfg)
        .args(["config", "dump"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout);
    let config_line = stdout
        .lines()
        .find(|l| l.starts_with("config_path:"))
        .expect("config_path line present");
    assert!(
        config_line.contains("# from: flag"),
        "config_path should attribute to flag when --config is set; got: {config_line}"
    );
}

#[test]
fn flag_overrides_file_with_source_attribution() {
    let dir = tempdir().unwrap();
    let cfg = write_config(dir.path(), "base_url: https://from-file.example\n");

    aviso_with_config(&cfg)
        .args(["--base-url", "https://from-flag.example", "config", "dump"])
        .assert()
        .success()
        .stdout(contains("https://from-flag.example"))
        .stdout(contains("# from: flag"));
}

#[test]
fn env_overrides_file_when_no_flag() {
    let dir = tempdir().unwrap();
    let cfg = write_config(dir.path(), "base_url: https://from-file.example\n");

    aviso_with_config(&cfg)
        .env("AVISO_BASE_URL", "https://from-env.example")
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("https://from-env.example"))
        .stdout(contains("# from: env"));
}

#[test]
fn json_form_emits_valid_json_with_source_fields() {
    let dir = tempdir().unwrap();
    let cfg = write_config(dir.path(), "base_url: https://from-file.example\n");

    aviso_with_config(&cfg)
        .args(["--json", "config", "dump"])
        .assert()
        .success()
        .stdout(contains("\"config_path\":"))
        .stdout(contains("\"source\":\"file\""));
}

#[test]
fn redact_masks_token_when_provider_set() {
    let dir = tempdir().unwrap();
    let cfg = write_config(
        dir.path(),
        "base_url: https://example\nauth:\n  bearer_token: super-secret-token\n",
    );

    let assertion = aviso_with_config(&cfg)
        .args(["config", "dump", "--redact"])
        .assert()
        .success();
    let output = assertion.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("super-secret-token"),
        "token must be masked under --redact; got: {stdout}"
    );
    assert!(
        stdout.contains("<set; redacted>"),
        "redacted marker missing under --redact; got: {stdout}"
    );
}

#[test]
fn dump_does_not_show_credentials_embedded_in_the_base_url() {
    let dir = tempdir().unwrap();
    let cfg = write_config(
        dir.path(),
        "base_url: https://operator:hunter2@aviso.example.org/api/\n",
    );
    for extra in [&[][..], &["--json"][..]] {
        let assertion = aviso_with_config(&cfg)
            .args(["config", "dump"])
            .args(extra)
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).into_owned();
        assert!(
            stdout.contains("https://aviso.example.org/api/"),
            "got: {stdout}"
        );
        assert!(!stdout.contains("hunter2"), "got: {stdout}");
        assert!(!stdout.contains("operator"), "got: {stdout}");
    }

    // A value that does not parse as a URL is not echoed either: the part
    // that breaks it may sit right next to the password.
    let cfg = write_config(dir.path(), "base_url: https://operator:hunter2@\n");
    let assertion = aviso_with_config(&cfg)
        .args(["config", "dump"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout).into_owned();
    assert!(stdout.contains("<unparseable url>"), "got: {stdout}");
    assert!(!stdout.contains("hunter2"), "got: {stdout}");
}

#[test]
fn dump_without_auth_block_shows_unset_provider() {
    let dir = tempdir().unwrap();
    let cfg = write_config(dir.path(), "base_url: https://example\n");

    aviso_with_config(&cfg)
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("provider: <unset>"));
}

#[test]
fn yaml_unknown_field_in_config_errors_with_file_path() {
    let dir = tempdir().unwrap();
    let cfg = write_config(dir.path(), "bogus_top_level_key: 1\n");

    aviso_with_config(&cfg)
        .args(["config", "dump"])
        .assert()
        .failure()
        .code(1)
        .stderr(contains("bogus_top_level_key").or(contains("unknown field")));
}

#[test]
fn dump_reflects_listeners_count_and_event_names() {
    let dir = tempdir().unwrap();
    let cfg = write_config(
        dir.path(),
        "listeners:\n  - event: mars\n    identifiers:\n      class: od\n  - event: era5\n",
    );

    aviso_with_config(&cfg)
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("listeners_count: 2"))
        .stdout(contains("event: mars"))
        .stdout(contains("event: era5"));
}
