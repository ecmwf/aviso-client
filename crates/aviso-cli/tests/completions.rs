//! Integration tests for `aviso completions <SHELL>`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on assert_cmd assertions is the expected diagnostic"
)]

mod common;

use predicates::str::contains;

use common::aviso;

#[test]
fn bash_completions_succeed_and_name_the_binary() {
    aviso()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(contains("aviso"));
}

#[test]
fn zsh_completions_succeed() {
    aviso()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicates::function::function(|stdout: &str| {
            !stdout.is_empty()
        }));
}

#[test]
fn fish_completions_succeed() {
    aviso().args(["completions", "fish"]).assert().success();
}

#[test]
fn powershell_completions_succeed() {
    aviso()
        .args(["completions", "powershell"])
        .assert()
        .success();
}

#[test]
fn elvish_completions_succeed() {
    aviso().args(["completions", "elvish"]).assert().success();
}

#[test]
fn unknown_shell_rejected() {
    aviso()
        .args(["completions", "tcsh"])
        .assert()
        .failure()
        .code(2);
}
