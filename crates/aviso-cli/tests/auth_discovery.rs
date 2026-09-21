// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Integration tests for credential discovery.
//!
//! Checks the order the binary consults its four sources, and that a
//! credentials file which exists but cannot be used is reported rather than
//! skipped. `aviso config dump` names the winning source, so these tests read
//! that rather than inspecting a request.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use std::path::{Path, PathBuf};

use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use tempfile::tempdir;

use common::aviso;

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write fixture");
    path
}

#[test]
fn credentials_file_is_used_when_nothing_else_is_set() {
    let dir = tempdir().unwrap();
    let credentials = write(
        dir.path(),
        "credentials.yaml",
        "bearer:\n  token: from-file\n",
    );

    aviso()
        .env("AVISO_CREDENTIALS_FILE", &credentials)
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("source: credentials file"));
}

#[test]
fn config_file_auth_beats_the_credentials_file() {
    let dir = tempdir().unwrap();
    let config = write(
        dir.path(),
        "config.yaml",
        "auth:\n  bearer_token: from-config\n",
    );
    let credentials = write(
        dir.path(),
        "credentials.yaml",
        "bearer:\n  token: from-file\n",
    );

    aviso()
        .env("AVISO_CREDENTIALS_FILE", &credentials)
        .arg("--config")
        .arg(&config)
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("source: config file"));
}

#[test]
fn environment_beats_the_config_file() {
    let dir = tempdir().unwrap();
    let config = write(
        dir.path(),
        "config.yaml",
        "auth:\n  bearer_token: from-config\n",
    );
    let credentials = write(
        dir.path(),
        "credentials.yaml",
        "bearer:\n  token: from-file\n",
    );

    aviso()
        .env("AVISO_CREDENTIALS_FILE", &credentials)
        .env("AVISO_TOKEN", "from-environment")
        .arg("--config")
        .arg(&config)
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("source: environment"));
}

#[test]
fn the_token_flag_beats_the_environment() {
    let dir = tempdir().unwrap();
    let credentials = write(
        dir.path(),
        "credentials.yaml",
        "bearer:\n  token: from-file\n",
    );

    aviso()
        .env("AVISO_CREDENTIALS_FILE", &credentials)
        .env("AVISO_TOKEN", "from-environment")
        .args(["--token", "from-flag", "config", "dump"])
        .assert()
        .success()
        .stdout(contains("source: flag"));
}

#[test]
fn no_credentials_anywhere_reports_no_source() {
    aviso()
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("provider: <unset>"))
        .stdout(contains("source: <unset>"));
}

#[test]
fn a_credentials_file_that_cannot_be_parsed_is_reported() {
    let dir = tempdir().unwrap();
    let credentials = write(dir.path(), "credentials.yaml", "bearer:\n  toke: typo\n");

    aviso()
        .env("AVISO_CREDENTIALS_FILE", &credentials)
        .args(["config", "dump"])
        .assert()
        .failure()
        .stderr(contains("credentials file"));
}

#[test]
fn a_missing_credentials_file_is_not_an_error() {
    let dir = tempdir().unwrap();

    aviso()
        .env("AVISO_CREDENTIALS_FILE", dir.path().join("absent.yaml"))
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("source: <unset>"));
}

#[test]
fn the_json_form_reports_the_source_too() {
    let dir = tempdir().unwrap();
    let credentials = write(
        dir.path(),
        "credentials.yaml",
        "bearer:\n  token: from-file\n",
    );

    let assertion = aviso()
        .env("AVISO_CREDENTIALS_FILE", &credentials)
        .args(["config", "dump", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assertion.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(parsed["auth"]["source"], "credentials file");
    assert_eq!(parsed["auth"]["provider_set"], true);
}

#[test]
fn the_credentials_file_does_not_leak_its_token_into_the_dump() {
    let dir = tempdir().unwrap();
    let credentials = write(
        dir.path(),
        "credentials.yaml",
        "bearer:\n  token: super-secret-value\n",
    );

    aviso()
        .env("AVISO_CREDENTIALS_FILE", &credentials)
        .args(["config", "dump"])
        .assert()
        .success()
        .stdout(contains("super-secret-value").not());
}
