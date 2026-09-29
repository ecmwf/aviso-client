// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The '`--from` value formats' section of the CLI configuration guide lists
//! the forms the `aviso` binary accepts for `--from` and `--until`.
//!
//! Runs the binary without a server: a value it parses gets as far as
//! listener resolution, one it does not is refused as a parse error. Lives in
//! this unpublished crate because it reads the repository's docs.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on fixtures is the expected diagnostic"
)]

use aviso_e2e::isolated_aviso_command;

/// The numbered list of the section, as (form, examples) pairs. The first
/// entry is the pure-digit sequence id, whose examples are its forms.
fn documented_forms() -> Vec<(String, Vec<String>)> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/src/cli/configuration.md"
    );
    let page = std::fs::read_to_string(path).expect("the configuration guide is readable");
    let section = page
        .split("## `--from` value formats")
        .nth(1)
        .and_then(|rest| rest.split("\n#").next())
        .expect("the guide has a '`--from` value formats' section");
    section
        .lines()
        .filter(|line| {
            line.split_once(". ")
                .is_some_and(|(n, _)| n.parse::<u8>().is_ok())
        })
        .map(|line| {
            let mut tokens: Vec<String> = line
                .split('`')
                .skip(1)
                .step_by(2)
                .map(|token| token.trim_matches('"').to_string())
                .collect();
            if line.contains("**Pure digits**") {
                ("pure digits".to_string(), tokens)
            } else {
                let form = tokens.remove(0);
                (form, tokens)
            }
        })
        .collect()
}

/// Fills a documented date form with a concrete value.
fn fill(form: &str) -> String {
    form.replacen("YYYY-MM-DD", "2026-05-01", 1)
        .replacen("HH", "14", 1)
        .replacen("MM", "30", 1)
        .replacen("SS", "15", 1)
        .replacen("ffffff", "123456", 1)
}

/// Runs `aviso replay` with `flag value` and returns its stderr.
fn stderr_for(flag: &str, value: &str) -> String {
    let mut args = vec![
        "--base-url",
        "http://127.0.0.1:1",
        "replay",
        "--listener",
        "none",
    ];
    match flag {
        "--from" => args.extend(["--from", value]),
        _ => args.extend(["--from", "2000-01-01", flag, value]),
    }
    let output = isolated_aviso_command()
        .args(args)
        .output()
        .expect("the aviso binary runs");
    String::from_utf8(output.stderr).unwrap()
}

/// Asserts the binary accepts `value` for `flag`.
fn assert_accepted(flag: &str, value: &str) {
    let stderr = stderr_for(flag, value);
    assert!(
        stderr.contains("no listener with name `none`"),
        "{flag} {value:?} was not accepted: {stderr}"
    );
}

#[test]
fn documented_forms_are_the_accepted_forms() {
    let forms = documented_forms();
    let refused = stderr_for("--from", "not-a-position");
    let accepted: Vec<String> = refused
        .split("accepted forms: ")
        .nth(1)
        .expect("a refused value lists the accepted forms")
        .trim()
        .split("; ")
        .map(|form| {
            form.trim_end_matches(" (quotes required)")
                .trim_matches('"')
                .to_string()
        })
        .collect();

    assert_eq!(forms.len(), accepted.len(), "{forms:?} vs {accepted:?}");
    assert_eq!(
        forms[0].0, "pure digits",
        "the list starts with sequence ids"
    );
    assert!(
        accepted[0].starts_with("pure-digit sequence id"),
        "{accepted:?}"
    );
    assert!(!forms[0].1.is_empty(), "sequence ids have examples");
    for (form, listed) in forms.iter().zip(&accepted).skip(1) {
        assert_eq!(
            &form.0, listed,
            "the guide and the binary list the same forms"
        );
    }

    for flag in ["--from", "--until"] {
        for (form, examples) in &forms {
            if form != "pure digits" {
                assert_accepted(flag, &fill(form));
            }
            for example in examples {
                assert_accepted(flag, example);
            }
        }
    }
}
