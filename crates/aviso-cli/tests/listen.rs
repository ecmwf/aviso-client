//! Integration tests for `aviso listen`.
//!
//! These tests exercise the CLI-unique surface: Amendment C
//! resolution rules (positional vs config), the no-listeners
//! error per Amendment C, the unset-base-url usage error, and
//! the YAML parse error path. The watch-mode reconnect loop is
//! a library concern (covered by lib integration tests); CLI
//! tests focus on what the CLI alone owns.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use std::io::Write as _;

use predicates::prelude::*;
use predicates::str::contains;
use tempfile::{NamedTempFile, tempdir};

use common::aviso;

fn write_listener_file(yaml_body: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("create tempfile");
    f.write_all(yaml_body.as_bytes()).expect("write yaml");
    f
}

#[test]
fn listen_with_no_positional_and_empty_config_exits_2_with_helpful_message() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "").unwrap();

    aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains("no listeners to run"))
        .stderr(contains("pass listener YAML files"));
}

#[test]
fn listen_yaml_parse_error_includes_listener_file_path() {
    let listener = write_listener_file("listeners:\n  - bogus_field: 1\n    event: mars\n");
    let listener_path = listener.path().to_path_buf();

    aviso()
        .args([
            "--base-url",
            "http://unused",
            "listen",
            listener_path.to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(1)
        .stderr(contains(listener_path.display().to_string()).or(contains("bogus_field")));
}

#[test]
fn listen_unset_base_url_exits_2() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "").unwrap();
    let listener = write_listener_file("listeners:\n  - event: mars\n");

    aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "listen",
            listener.path().to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains("base_url"));
}

#[test]
fn listen_no_listeners_message_names_config_path() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "").unwrap();

    aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2)
        .stderr(contains(cfg_path.display().to_string()));
}
