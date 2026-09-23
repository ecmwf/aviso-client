// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The resolver behind `from_file`, `from_environment` and the config dumps.
//!
//! Every test points the config-file location and the credential sources at
//! a temporary directory, so the result does not depend on the machine.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on fixture setup are the expected diagnostics"
)]

#[path = "common/env.rs"]
mod env;

use aviso::resolve::{CodeInputs, EnvAddress, ResolvedSettings, Source, resolve};
use aviso::{AvisoClient, AvisoClientBuilder, ClientError};
use env::{Sources, write_config};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn resolve_here(inputs: &CodeInputs) -> ResolvedSettings {
    resolve(
        inputs,
        &aviso::auth::DiscoveryPaths::from_env(),
        EnvAddress::Read,
    )
    .expect("resolve")
    .settings
}

#[test]
fn the_address_comes_from_code_then_the_environment_then_the_file() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "base_url: https://file.example.org\n");
    let _sources = Sources::in_dir(dir.path());

    let from_file = resolve_here(&CodeInputs::default());
    let url = from_file.base_url.expect("file supplies it");
    assert_eq!(url.value, "https://file.example.org/");
    assert_eq!(
        url.source,
        Source::ConfigFile(dir.path().join("config.yaml"))
    );

    // SAFETY: the Sources guard holds ENV_LOCK and restores this variable.
    unsafe { std::env::set_var("AVISO_BASE_URL", "https://env.example.org") };
    let from_env = resolve_here(&CodeInputs::default());
    let url = from_env.base_url.expect("env supplies it");
    assert_eq!(url.value, "https://env.example.org/");
    assert_eq!(url.source, Source::Environment("AVISO_BASE_URL"));

    let inputs = CodeInputs {
        base_url: Some("https://code.example.org".into()),
        ..CodeInputs::default()
    };
    let from_code = resolve_here(&inputs);
    let url = from_code.base_url.expect("code supplies it");
    assert_eq!(url.value, "https://code.example.org/");
    assert_eq!(url.source, Source::Code);
    Ok(())
}

#[test]
fn a_client_built_from_the_environment_uses_the_environment_address() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "base_url: https://file.example.org\n");
    let _sources = Sources::in_dir(dir.path());
    // SAFETY: as above.
    unsafe { std::env::set_var("AVISO_BASE_URL", "https://env.example.org") };

    let client = AvisoClientBuilder::from_environment(&CodeInputs::default())?.build()?;
    assert_eq!(client.display_base_url(), "https://env.example.org/");
    Ok(())
}

#[test]
fn the_report_hides_the_secret_and_names_each_source() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: https://alice:hunter2@file.example.org\ntimeout: 7s\nauth:\n  bearer_token: from-file\n",
    );
    let _sources = Sources::in_dir(dir.path());

    let report = resolve_here(&CodeInputs::default());
    let shown = format!("{report:?}");
    assert!(!shown.contains("hunter2"), "got: {shown}");
    assert!(!shown.contains("from-file"), "got: {shown}");
    let url = report.base_url.expect("set");
    assert_eq!(url.value, "https://file.example.org/");
    assert_eq!(
        report.timeout.value,
        Some(std::time::Duration::from_secs(7))
    );
    assert!(matches!(report.timeout.source, Source::ConfigFile(_)));
    assert_eq!(report.heartbeat_interval.value, None);
    assert_eq!(report.heartbeat_interval.source, Source::Default);
    let auth = report.auth.expect("found");
    assert_eq!(auth.kind, "bearer");
    assert!(matches!(auth.source, Source::ConfigFile(_)));
    assert_eq!(auth.refused, None);
    assert_eq!(report.config_file, Some(dir.path().join("config.yaml")));
    Ok(())
}

#[test]
fn the_report_says_when_a_found_credential_would_be_refused() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: http://public.example.org\nauth:\n  bearer_token: from-file\n",
    );
    let _sources = Sources::in_dir(dir.path());

    let report = resolve_here(&CodeInputs::default());
    let auth = report.auth.expect("found");
    let refused = auth.refused.expect("refused for plain http");
    assert!(refused.contains("refused"), "got: {refused}");
    assert!(refused.contains("public.example.org"), "got: {refused}");

    // Named in code, the credential is reported as such and never refused.
    let inputs = CodeInputs {
        auth_kind: Some("bearer"),
        ..CodeInputs::default()
    };
    let report = resolve_here(&inputs);
    let auth = report.auth.expect("named");
    assert_eq!(auth.source, Source::Code);
    assert_eq!(auth.refused, None);
    Ok(())
}

#[test]
fn no_address_anywhere_is_reported_as_none_and_refused_at_build() -> TestResult {
    let dir = tempfile::tempdir()?;
    let _sources = Sources::in_dir(dir.path());

    let report = resolve_here(&CodeInputs::default());
    assert_eq!(report.base_url, None);
    assert_eq!(report.config_file, None);

    let error = AvisoClientBuilder::from_environment(&CodeInputs::default())?
        .build()
        .unwrap_err();
    assert!(matches!(error, ClientError::Config(_)), "got {error:?}");
    assert!(error.to_string().contains("AVISO_BASE_URL"), "got {error}");
    Ok(())
}

#[test]
fn from_file_reads_the_address_from_the_file_only() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "base_url: https://file.example.org\n");
    let _sources = Sources::in_dir(dir.path());
    // SAFETY: as above.
    unsafe { std::env::set_var("AVISO_BASE_URL", "https://env.example.org") };

    let client = AvisoClientBuilder::from_file()?.build()?;
    assert_eq!(client.display_base_url(), "https://file.example.org/");
    let client = AvisoClientBuilder::from_file_at(dir.path().join("config.yaml"))?.build()?;
    assert_eq!(client.display_base_url(), "https://file.example.org/");

    let ignored = resolve(
        &CodeInputs::default(),
        &aviso::auth::DiscoveryPaths::from_env(),
        EnvAddress::Ignore,
    )?
    .settings;
    assert!(matches!(
        ignored.base_url.map(|u| u.source),
        Some(Source::ConfigFile(_))
    ));
    Ok(())
}

#[test]
fn from_environment_applies_the_values_passed_in_code() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: https://file.example.org\ntimeout: 7s\nheartbeat_interval: 9s\n",
    );
    let _sources = Sources::in_dir(dir.path());

    let inputs = CodeInputs {
        base_url: Some("https://code.example.org/api".into()),
        timeout: Some(std::time::Duration::from_secs(3)),
        ..CodeInputs::default()
    };
    let builder = AvisoClientBuilder::from_environment(&inputs)?;
    assert!(
        format!("{builder:?}").contains("timeout: Some(3s)"),
        "got {builder:?}"
    );
    let client = builder.build()?;
    assert_eq!(client.display_base_url(), "https://code.example.org/api/");

    let report = resolve_here(&inputs);
    assert_eq!(
        report.base_url.expect("set").value,
        "https://code.example.org/api/"
    );
    assert_eq!(report.timeout.source, Source::Code);
    assert_eq!(
        report.heartbeat_interval.value,
        Some(std::time::Duration::from_secs(9))
    );
    Ok(())
}

#[test]
fn the_resolution_debug_output_hides_userinfo() -> TestResult {
    let dir = tempfile::tempdir()?;
    let _sources = Sources::in_dir(dir.path());
    // SAFETY: as above.
    unsafe { std::env::set_var("AVISO_BASE_URL", "http://op:hunter2@127.0.0.1:1") };

    let resolution = resolve(
        &CodeInputs::default(),
        &aviso::auth::DiscoveryPaths::from_env(),
        EnvAddress::Read,
    )?;
    let shown = format!("{resolution:?}");
    assert!(!shown.contains("hunter2"), "got: {shown}");
    assert!(shown.contains("127.0.0.1"), "got: {shown}");
    Ok(())
}

#[test]
fn a_plain_builder_error_does_not_claim_to_have_looked_anywhere() {
    let error = AvisoClient::builder().build().unwrap_err();
    assert!(!error.to_string().contains("AVISO_BASE_URL"), "got {error}");
}

#[test]
fn the_report_displays_one_setting_per_line_without_the_secret() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: https://alice:hunter2@file.example.org\ntimeout: 7s\nauth:\n  bearer_token: from-file\n",
    );
    let _sources = Sources::in_dir(dir.path());

    let shown = resolve_here(&CodeInputs::default()).to_string();
    assert!(!shown.contains("hunter2"), "got: {shown}");
    assert!(!shown.contains("from-file"), "got: {shown}");
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines.len(), 6, "got: {shown}");
    assert!(lines[0].starts_with("base_url"), "got: {shown}");
    assert!(
        lines[0].contains("https://file.example.org/"),
        "got: {shown}"
    );
    assert!(lines[1].contains("bearer"), "got: {shown}");
    assert!(lines[2].contains("7s"), "got: {shown}");
    assert!(lines[3].ends_with("(default)"), "got: {shown}");
    Ok(())
}
