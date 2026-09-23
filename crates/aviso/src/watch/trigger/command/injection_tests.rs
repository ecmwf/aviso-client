// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Notification values reach the command trigger from whoever published
//! the notification. These tests hand the dispatcher values full of
//! shell metacharacters, in each of the three places an operator might
//! put a `{{ }}`, and check that the child received them as literal text
//! and that nothing else ran.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap/expect on dispatch results are the standard test diagnostics"
)]

use std::collections::BTreeMap;
use std::path::Path;

use super::{build_command_config, dispatch_command};
use crate::Notification;

/// A notification whose string values would all execute something if
/// the shell read them as code. `marker` is the file each one would
/// create.
fn hostile_notification(marker: &Path) -> Notification {
    let marker = marker.display();
    let mut identifier = BTreeMap::new();
    identifier.insert(
        "country".to_string(),
        serde_json::json!(format!("$(touch {marker})")),
    );
    identifier.insert("step".to_string(), serde_json::json!(12));
    Notification {
        event_type: format!("mars`touch {marker}`"),
        sequence: 42,
        identifier,
        payload: serde_json::json!({
            "location": format!("south; touch {marker} #"),
            "closer": format!("x'; touch {marker} ; echo '"),
        }),
        cloudevent: None,
    }
}

struct Run {
    output: String,
    marker_exists: bool,
}

/// Runs `template` with `> out` appended, against the hostile
/// notification, and reports what the child wrote and whether any of
/// the embedded commands executed.
async fn run(template: &str) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("injected");
    let out = dir.path().join("out");
    let cfg = build_command_config(format!("{template} > '{}'", out.display()));
    let result = dispatch_command(&cfg, None, &hostile_notification(&marker)).await;
    assert!(result.is_ok(), "dispatch failed: {result:?}");
    Run {
        output: std::fs::read_to_string(&out).expect("read out"),
        marker_exists: marker.exists(),
    }
}

#[tokio::test]
async fn a_bare_value_is_one_literal_argument() {
    let run = run("printf '%s' {{ notification.payload.location }}").await;
    assert!(
        run.output.starts_with("south; touch "),
        "got: {}",
        run.output
    );
    assert!(run.output.ends_with(" #"), "got: {}", run.output);
    assert!(!run.marker_exists, "the embedded command ran");
}

#[tokio::test]
async fn a_value_inside_double_quotes_cannot_substitute_a_command() {
    let run = run(r#"printf '%s' "{{ notification.identifier.country }}""#).await;
    assert!(run.output.starts_with("$(touch "), "got: {}", run.output);
    assert!(
        !run.marker_exists,
        "command substitution ran inside double quotes"
    );
}

#[tokio::test]
async fn a_value_inside_single_quotes_cannot_close_them() {
    // The shape of the documented curl example: an operator wraps the
    // payload in single quotes and a value contains a single quote.
    let run = run("printf '%s' -d '{{ notification.payload.closer }}'").await;
    assert!(
        run.output.starts_with("-dx'; touch "),
        "got: {}",
        run.output
    );
    assert!(run.output.ends_with(" ; echo '"), "got: {}", run.output);
    assert!(!run.marker_exists, "the value closed the operator's quote");
}

#[tokio::test]
async fn the_event_type_is_data_too() {
    let run = run("printf '%s' {{ notification.event_type }}").await;
    assert!(run.output.starts_with("mars`touch "), "got: {}", run.output);
    assert!(!run.marker_exists, "backticks in event_type ran");
}

#[tokio::test]
async fn a_whole_json_object_survives_as_one_argument() {
    let run = run("printf '%s' {{ notification.payload }}").await;
    let parsed: serde_json::Value = serde_json::from_str(&run.output).expect("valid JSON");
    let location = parsed["location"].as_str().expect("location is a string");
    assert!(location.starts_with("south; touch "), "got: {location}");
    assert!(!run.marker_exists);
}

#[tokio::test]
async fn plain_values_render_the_way_they_always_did() {
    let run = run("printf '%s|%s|%s' {{ notification.identifier.step }} '{{ notification.sequence }}' \"{{ notification.sequence }}\"").await;
    assert_eq!(run.output, "12|42|42");
}

#[tokio::test]
async fn env_values_are_the_operators_own_and_stay_verbatim() {
    // PATH is always set. An env value is inserted as written, so the
    // operator's own quoting around it is what the shell sees.
    let run = run("printf '%s' '{{ env.PATH }}'").await;
    assert_eq!(run.output, std::env::var("PATH").unwrap());
}

/// Runs `template` and returns what the child wrote, with no hostile
/// values involved.
async fn output_of(template: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("out");
    let cfg = build_command_config(format!("{template} > '{}'", out.display()));
    let result = dispatch_command(&cfg, None, &hostile_notification(&dir.path().join("m"))).await;
    assert!(result.is_ok(), "dispatch failed: {result:?}");
    std::fs::read_to_string(&out).expect("read out")
}

#[tokio::test]
async fn the_child_does_not_see_the_listeners_credentials() {
    // The test process may or may not have these set. Either way the
    // child must report every one of them as unset.
    let out = output_of(
        "printf '%s|%s|%s' \"${AVISO_TOKEN-unset}\" \"${AVISO_USERNAME-unset}\" \"${AVISO_PASSWORD-unset}\"",
    )
    .await;
    assert_eq!(out, "unset|unset|unset");
}

#[tokio::test]
async fn a_variable_the_template_read_is_not_passed_on_by_name() {
    // HOME is the one variable a test process reliably has that the
    // shell does not set for itself when missing (it defaults PATH).
    if std::env::var_os("HOME").is_none() {
        return;
    }
    let out = output_of("printf '%s|%s' '{{ env.HOME }}' \"${HOME-unset}\"").await;
    let home = std::env::var("HOME").unwrap();
    assert_eq!(out, format!("{home}|unset"));
}

#[tokio::test]
async fn an_explicit_env_entry_still_reaches_the_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("out");
    let mut cfg = build_command_config(format!(
        "printf '%s' \"${{AVISO_TOKEN-unset}}\" > '{}'",
        out.display()
    ));
    cfg.env
        .insert("AVISO_TOKEN".to_string(), "given-on-purpose".to_string());
    let result = dispatch_command(&cfg, None, &hostile_notification(&dir.path().join("m"))).await;
    assert!(result.is_ok(), "dispatch failed: {result:?}");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "given-on-purpose");
}

#[tokio::test]
async fn an_apostrophe_in_a_comment_does_not_open_a_quote() {
    // A multi-line command with a comment is an ordinary YAML shape. The
    // apostrophe in the comment must not make the next value look like
    // it lands inside single quotes.
    let run = run("# don't run this twice\nprintf '%s' {{ notification.payload.location }}").await;
    assert!(
        run.output.starts_with("south; touch "),
        "got: {}",
        run.output
    );
    assert!(
        !run.marker_exists,
        "the value after the comment ran as code"
    );
}

#[tokio::test]
async fn a_value_inside_a_comment_is_ignored() {
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("injected");
    let out = dir.path().join("out");
    let cfg = build_command_config(format!(
        "printf '%s' ok > '{}' # {{{{ notification.payload.location }}}}",
        out.display()
    ));
    let result = dispatch_command(&cfg, None, &hostile_notification(&marker)).await;
    assert!(result.is_ok(), "dispatch failed: {result:?}");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "ok");
    assert!(!marker.exists());
}

#[tokio::test]
async fn a_comment_after_a_line_continuation_is_still_a_comment() {
    let run =
        run("printf '%s' ok \\\n# don't\nprintf '%s' {{ notification.payload.location }}").await;
    // The first printf writes to the child's stdout, which is dropped; the
    // redirect run() appends applies to the last command only.
    assert!(
        run.output.starts_with("south; touch "),
        "got: {}",
        run.output
    );
    assert!(
        !run.marker_exists,
        "the value after the comment ran as code"
    );
}

#[tokio::test]
async fn a_value_after_a_here_document_is_refused_rather_than_guessed() {
    // The here-document body is opaque to sh but would confuse a tracker
    // that read it as shell text. Instead of guessing, the render fails
    // and nothing runs.
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("injected");
    let cfg = build_command_config(
        "cat <<EOF >/dev/null\ncan't\nEOF\nprintf '%s' {{ notification.payload.location }}",
    );
    let result = dispatch_command(&cfg, None, &hostile_notification(&marker)).await;
    let Err(crate::watch::TriggerError::Template { kind, field, .. }) = result else {
        unreachable!("expected a template error, got {result:?}");
    };
    assert_eq!(
        kind,
        crate::watch::TemplateErrorKind::ValueAfterUnsupportedShellSyntax
    );
    assert_eq!(field, "a here-document");
    assert!(!marker.exists());
}

#[tokio::test]
async fn a_value_right_after_a_dollar_is_refused() {
    // `"${{ v }}"` with a value of `(touch x)` would be `"$(touch x)"`.
    let dir = tempfile::tempdir().expect("tempdir");
    let marker = dir.path().join("injected");
    let mut n = hostile_notification(&marker);
    n.payload = serde_json::json!({ "v": format!("(touch {})", marker.display()) });
    let cfg = build_command_config("printf '%s' \"${{ notification.payload.v }}\"");
    let result = dispatch_command(&cfg, None, &n).await;
    let Err(crate::watch::TriggerError::Template { kind, .. }) = result else {
        unreachable!("expected a template error, got {result:?}");
    };
    assert_eq!(
        kind,
        crate::watch::TemplateErrorKind::ValueAfterUnsupportedShellSyntax
    );
    assert!(!marker.exists(), "the joined $( ran");
}
