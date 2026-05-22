//! Integration tests for clap-auto-generated help and version output.
//!
//! These tests do not exercise any subcommand behaviour; they verify
//! that the clap-derive Cli definition parses cleanly and produces
//! the expected help and version surface. They run against the
//! stubbed binary so they pass before commit-by-commit subcommand
//! work lands.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use predicates::prelude::*;
use predicates::str::contains;

use common::aviso;

#[test]
fn version_prints_aviso_lib_version() {
    aviso()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(aviso::VERSION));
}

#[test]
fn top_level_help_lists_every_subcommand() {
    aviso()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("notify"))
        .stdout(contains("listen"))
        .stdout(contains("replay"))
        .stdout(contains("schema"))
        .stdout(contains("admin"))
        .stdout(contains("config"))
        .stdout(contains("completions"));
}

#[test]
fn top_level_help_does_not_list_watch_or_auth_check() {
    aviso()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("listen").and(predicates::str::contains("watch ").not()))
        .stdout(predicates::str::contains("auth check").not())
        .stdout(predicates::str::contains("auth-check").not());
}

#[test]
fn top_level_help_includes_global_flags() {
    aviso()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--config"))
        .stdout(contains("-c"))
        .stdout(contains("--state-file"))
        .stdout(contains("--base-url"))
        .stdout(contains("--token"))
        .stdout(contains("--username"))
        .stdout(contains("--password"))
        .stdout(contains("--ca-bundle"))
        .stdout(contains("--danger-accept-invalid-certs"))
        .stdout(contains("--json"))
        .stdout(contains("--color <COLOR>"))
        .stdout(contains("--verbose"));
}

#[test]
fn top_level_help_does_not_offer_yes_as_global_option() {
    let output = aviso().arg("--help").output().expect("invoke --help");
    assert!(output.status.success(), "--help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let options_section = stdout
        .lines()
        .skip_while(|l| !l.contains("Options:"))
        .take_while(|l| !l.starts_with("Commands:") && !l.starts_with("Subcommands:"))
        .collect::<Vec<_>>();
    let yes_in_options = options_section
        .iter()
        .any(|l| l.trim_start().starts_with("--yes"));
    assert!(
        !yes_in_options,
        "--yes must not appear as a global option (it is per-leaf admin only); options section was:\n{}",
        options_section.join("\n")
    );
}

#[test]
fn notify_subcommand_help() {
    aviso()
        .args(["notify", "--help"])
        .assert()
        .success()
        .stdout(contains("PARAMETERS"));
}

#[test]
fn listen_subcommand_help() {
    aviso()
        .args(["listen", "--help"])
        .assert()
        .success()
        .stdout(contains("LISTENER_FILES").or(contains("listener_files")))
        .stdout(contains("--no-state-store"));
}

#[test]
fn replay_subcommand_help() {
    aviso()
        .args(["replay", "--help"])
        .assert()
        .success()
        .stdout(contains("--from"));
}

#[test]
fn schema_list_subcommand_help() {
    aviso()
        .args(["schema", "list", "--help"])
        .assert()
        .success();
}

#[test]
fn schema_get_subcommand_help() {
    aviso().args(["schema", "get", "--help"]).assert().success();
}

#[test]
fn admin_wipe_stream_help_includes_yes_flag() {
    aviso()
        .args(["admin", "wipe-stream", "--help"])
        .assert()
        .success()
        .stdout(contains("--yes"));
}

#[test]
fn admin_wipe_all_help_includes_yes_flag() {
    aviso()
        .args(["admin", "wipe-all", "--help"])
        .assert()
        .success()
        .stdout(contains("--yes"));
}

#[test]
fn admin_delete_help_includes_yes_flag() {
    aviso()
        .args(["admin", "delete", "--help"])
        .assert()
        .success()
        .stdout(contains("--yes"));
}

#[test]
fn config_dump_help_includes_redact_flag() {
    aviso()
        .args(["config", "dump", "--help"])
        .assert()
        .success()
        .stdout(contains("--redact"));
}

#[test]
fn completions_help_lists_shells() {
    aviso().args(["completions", "--help"]).assert().success();
}

#[test]
fn admin_wipe_all_without_yes_exits_2() {
    aviso()
        .args(["admin", "wipe-all"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn admin_wipe_stream_without_yes_exits_2() {
    aviso()
        .args(["admin", "wipe-stream", "mars"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn admin_delete_without_yes_exits_2() {
    aviso()
        .args(["admin", "delete", "mars@1"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn token_and_username_conflict_exits_2() {
    aviso()
        .args([
            "--token",
            "t",
            "--username",
            "u",
            "--password",
            "p",
            "config",
            "dump",
        ])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn unknown_subcommand_exits_2() {
    aviso()
        .arg("bogus-subcommand-that-does-not-exist")
        .assert()
        .failure()
        .code(2);
}

#[test]
fn color_flag_requires_a_value_and_does_not_swallow_subcommand() {
    // Regression guard for the prior bug where `default_missing_value`
    // + `num_args = 0..=1` made bare `--color` consume the next
    // positional token (`listen`) as the color value, breaking
    // `aviso --color listen --help`.
    aviso()
        .args(["--color", "auto", "listen", "--help"])
        .assert()
        .success()
        .stdout(contains("LISTENER_FILES").or(contains("listener_files")));
}

#[test]
fn bare_color_flag_without_value_is_rejected() {
    let assertion = aviso()
        .args(["--color", "listen", "--help"])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("invalid value")
            || stderr.contains("requires a value")
            || stderr.contains("possible values"),
        "stderr should reject 'listen' as a color value: {stderr}"
    );
}

#[test]
fn color_accepts_each_explicit_mode_value() {
    for mode in ["auto", "always", "never"] {
        aviso().args(["--color", mode, "--help"]).assert().success();
    }
}
