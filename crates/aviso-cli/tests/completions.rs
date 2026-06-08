// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

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
