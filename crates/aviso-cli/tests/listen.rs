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

#[test]
fn listen_no_listeners_with_no_positional_does_not_claim_section_absent() {
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "listeners: []\n").unwrap();

    let assertion = aviso()
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
        .code(2);
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        !stderr.contains("no `listeners:` section"),
        "stderr must not falsely claim the section is absent when it is present-but-empty; got: {stderr}"
    );
    assert!(
        stderr.contains("resolved to 0 entries")
            || stderr.contains("absent, present-but-empty, or commented out"),
        "stderr should describe the empty-resolution accurately; got: {stderr}"
    );
}

#[test]
fn listen_no_listeners_with_positional_yaml_attributes_to_positional_path_not_config() {
    let listener = write_listener_file("listeners: []\n");
    let listener_path = listener.path().to_path_buf();
    let dir = tempdir().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    std::fs::write(&cfg_path, "listeners:\n  - event: never-resolved\n").unwrap();

    let assertion = aviso()
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--base-url",
            "http://unused",
            "listen",
            listener_path.to_str().unwrap(),
        ])
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .failure()
        .code(2);
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains(listener_path.display().to_string().as_str()),
        "stderr should name the positional file path (the source of the empty resolution); got: {stderr}"
    );
    assert!(
        !stderr.contains(cfg_path.display().to_string().as_str()),
        "stderr must NOT name the config path when positional files were supplied (Amendment C: positional REPLACES global); got: {stderr}"
    );
    assert!(
        stderr.contains("ensure each positional listener file") || stderr.contains("non-empty"),
        "stderr should suggest fixing the positional file; got: {stderr}"
    );
}
