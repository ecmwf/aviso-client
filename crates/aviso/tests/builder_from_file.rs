// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Building a client from the config file.
//!
//! These tests go through the public builder and, where a request is involved,
//! assert what reaches a server. They set the config-file location through
//! `AVISO_CLIENT_CONFIG_FILE` and point the other credential sources at paths
//! that do not exist, so the result does not depend on the machine.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on fixture setup are the expected diagnostics"
)]

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use aviso::{AvisoClient, AvisoClientBuilder, ClientError};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Points every source at `dir` and restores the previous values on drop.
struct Sources {
    _guard: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl Sources {
    fn in_dir(dir: &Path) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let names = [
            "AVISO_TOKEN",
            "AVISO_USERNAME",
            "AVISO_PASSWORD",
            "AVISO_CLIENT_CONFIG_FILE",
            "AVISO_CREDENTIALS_FILE",
        ];
        let saved = names.iter().map(|k| (*k, std::env::var_os(k))).collect();
        // SAFETY: ENV_LOCK is held, so no other test in this binary reads or
        // writes these variables while they are changed.
        unsafe {
            for name in &names[..3] {
                std::env::remove_var(name);
            }
            std::env::set_var("AVISO_CLIENT_CONFIG_FILE", dir.join("config.yaml"));
            std::env::set_var(
                "AVISO_CREDENTIALS_FILE",
                dir.join("absent-credentials.yaml"),
            );
        }
        Self {
            _guard: guard,
            saved,
        }
    }
}

impl Drop for Sources {
    fn drop(&mut self) {
        // SAFETY: the lock is still held for the lifetime of this value.
        unsafe {
            for (name, value) in &self.saved {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

fn write_config(dir: &Path, body: &str) {
    std::fs::write(dir.join("config.yaml"), body).expect("write config");
}

fn schema_ok() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "status": "success", "schema": {}, "event_types": [], "total_schemas": 0
    }))
}

#[tokio::test]
async fn the_file_supplies_the_address_and_the_credential() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(schema_ok())
        .mount(&server)
        .await;
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        &format!(
            "base_url: {}\nauth:\n  bearer_token: from-file\n",
            server.uri()
        ),
    );
    let _sources = Sources::in_dir(dir.path());

    let client = AvisoClient::builder_from_file()?.build()?;
    client.schema().await?;

    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .map(|v| v.to_str().unwrap_or("")),
        Some("Bearer from-file")
    );
    Ok(())
}

#[tokio::test]
async fn a_setter_called_afterwards_replaces_what_the_file_said() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/schema"))
        .respond_with(schema_ok())
        .mount(&server)
        .await;
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: https://nowhere.example.org\nauth:\n  bearer_token: from-file\n",
    );
    let _sources = Sources::in_dir(dir.path());

    let client = AvisoClient::builder_from_file()?
        .base_url(server.uri())
        .auth(std::sync::Arc::new(aviso::auth::Bearer::new("from-code")?))
        .build()?;
    client.schema().await?;

    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 1, "code overrode the file's address");
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .map(|v| v.to_str().unwrap_or("")),
        Some("Bearer from-code"),
        "code overrode the file's credential"
    );
    Ok(())
}

#[test]
fn a_missing_default_file_sets_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    let _sources = Sources::in_dir(dir.path());

    let builder = AvisoClient::builder_from_file()?;
    let error = builder.build().unwrap_err();

    assert!(
        matches!(error, ClientError::Config(_)),
        "no base_url from anywhere is the ordinary build error: {error:?}"
    );
    Ok(())
}

#[test]
fn a_named_file_that_does_not_exist_is_an_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    let _sources = Sources::in_dir(dir.path());

    let error = AvisoClientBuilder::from_file_at(dir.path().join("absent.yaml")).unwrap_err();

    assert!(matches!(error, ClientError::Config(_)), "got {error:?}");
    Ok(())
}

#[test]
fn a_broken_default_file_is_an_error_rather_than_nothing() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "tls:\n  ca_bundel: [x.pem]\n");
    let _sources = Sources::in_dir(dir.path());

    let error = AvisoClient::builder_from_file().unwrap_err();

    assert!(error.to_string().contains("ca_bundel"), "got {error}");
    Ok(())
}

#[test]
fn a_found_credential_is_refused_for_a_plaintext_address_from_the_file() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: http://aviso.example.org\nauth:\n  bearer_token: from-file\n",
    );
    let _sources = Sources::in_dir(dir.path());

    // The refusal happens at build, against the address the client will use,
    // so a caller can still override the address before then.
    let error = AvisoClient::builder_from_file()?.build().unwrap_err();

    assert!(matches!(error, ClientError::Auth(_)), "got {error:?}");
    assert!(!error.to_string().contains("from-file"));
    Ok(())
}

#[test]
fn the_environment_beats_the_file_for_the_credential() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: https://aviso.example.org\nauth:\n  bearer_token: from-file\n",
    );
    let _sources = Sources::in_dir(dir.path());
    // SAFETY: ENV_LOCK is held by `_sources`.
    unsafe { std::env::set_var("AVISO_TOKEN", "from-environment") };

    let builder = AvisoClient::builder_from_file()?;
    let rendered = format!("{builder:?}");

    // The provider redacts its token, so identity is checked by type name:
    // the environment yields Env, the file yields Bearer.
    assert!(rendered.contains("Env"), "got {rendered}");
    Ok(())
}

#[test]
fn a_named_file_supplies_its_own_auth_block() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "base_url: https://default.example.org\n");
    let other = dir.path().join("other.yaml");
    std::fs::write(
        &other,
        "base_url: https://other.example.org\nauth:\n  bearer_token: from-other\n",
    )?;
    let _sources = Sources::in_dir(dir.path());

    let builder = AvisoClientBuilder::from_file_at(&other)?;
    let rendered = format!("{builder:?}");

    assert!(rendered.contains("other.example.org"), "got {rendered}");
    assert!(
        rendered.contains("Bearer"),
        "credential came from the named file: {rendered}"
    );
    Ok(())
}

#[test]
fn a_found_credential_is_refused_when_code_later_points_at_plaintext() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: https://aviso.example.org\nauth:\n  bearer_token: from-file\n",
    );
    let _sources = Sources::in_dir(dir.path());

    // The file's own address was fine; the override is not. The rule has to
    // apply to the address the client will actually use.
    let error = AvisoClient::builder_from_file()?
        .base_url("http://public.example.org")
        .build()
        .unwrap_err();

    assert!(matches!(error, ClientError::Auth(_)), "got {error:?}");
    assert!(!error.to_string().contains("from-file"));
    Ok(())
}

#[test]
fn a_found_credential_is_refused_when_only_code_supplies_a_plaintext_address() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "auth:\n  bearer_token: from-file\n");
    let _sources = Sources::in_dir(dir.path());

    let error = AvisoClient::builder_from_file()?
        .base_url("http://public.example.org")
        .build()
        .unwrap_err();

    assert!(matches!(error, ClientError::Auth(_)), "got {error:?}");
    Ok(())
}

#[test]
fn naming_the_credential_after_from_file_lifts_the_plaintext_rule() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "auth:\n  bearer_token: from-file\n");
    let _sources = Sources::in_dir(dir.path());

    AvisoClient::builder_from_file()?
        .base_url("http://public.example.org")
        .auth(std::sync::Arc::new(aviso::auth::Bearer::new("named")?))
        .build()?;
    AvisoClient::builder_from_file()?
        .base_url("http://public.example.org")
        .anonymous()
        .build()?;
    Ok(())
}

#[test]
fn a_found_credential_may_still_go_to_loopback_plaintext() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "auth:\n  bearer_token: from-file\n");
    let _sources = Sources::in_dir(dir.path());

    AvisoClient::builder_from_file()?
        .base_url("http://127.0.0.1:1")
        .build()?;
    Ok(())
}

#[test]
fn the_credential_comes_from_the_same_read_as_the_settings() -> TestResult {
    // Cannot stage a concurrent replace deterministically, so check the
    // mechanism: discovery is handed the text that was parsed, not the path.
    // If it reopened the path it would see the second file's token.
    let dir = tempfile::tempdir()?;
    write_config(
        dir.path(),
        "base_url: https://aviso.example.org\nauth:\n  bearer_token: first\n",
    );
    let _sources = Sources::in_dir(dir.path());
    let loaded = aviso::ClientSettings::read(&dir.path().join("config.yaml"))?;
    write_config(
        dir.path(),
        "base_url: https://aviso.example.org\nauth:\n  bearer_token: second\n",
    );
    let mut paths = aviso::auth::DiscoveryPaths::from_env();
    paths.config_file = Some(loaded.path.clone());
    paths.config_content = Some(loaded.content.clone());

    let found = aviso::auth::discover_with(&paths)?.expect("credential");
    let header =
        tokio::runtime::Runtime::new()?.block_on(found.provider().authorization_header())?;

    assert_eq!(header, "Bearer first", "the snapshot, not the current file");
    Ok(())
}

/// A throwaway self-signed certificate, or `None` when `openssl` is not on
/// the PATH. Enough to prove a PEM named by the file is read and parsed.
fn self_signed_pem(dir: &Path) -> Option<std::path::PathBuf> {
    let out = dir.join("ca.pem");
    let status = std::process::Command::new("openssl")
        .args([
            "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
        ])
        .args(["-subj", "/CN=aviso-test", "-keyout"])
        .arg(dir.join("ca.key"))
        .arg("-out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    status.success().then_some(out)
}

#[test]
fn timeouts_and_certificates_from_the_file_reach_the_builder() -> TestResult {
    let dir = tempfile::tempdir()?;
    // Without openssl the certificate half is not exercised; the test still
    // covers the durations rather than failing on a missing tool.
    let has_pem = self_signed_pem(dir.path()).is_some();
    let tls = if has_pem {
        "tls:\n  ca_bundle: [ca.pem]\n"
    } else {
        ""
    };
    write_config(
        dir.path(),
        &format!(
            "base_url: https://aviso.example.org\ntimeout: 7s\nheartbeat_interval: 11s\n{tls}"
        ),
    );
    let _sources = Sources::in_dir(dir.path());

    let builder = AvisoClient::builder_from_file()?;
    let rendered = format!("{builder:?}");

    assert!(rendered.contains("timeout: Some(7s)"), "got {rendered}");
    assert!(
        rendered.contains("heartbeat_interval: Some(11s)"),
        "got {rendered}"
    );
    if has_pem {
        assert!(
            rendered.contains("extra_root_certs_count: 1"),
            "got {rendered}"
        );
    }
    Ok(())
}

#[test]
fn a_missing_certificate_named_by_the_file_is_reported() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "tls:\n  ca_bundle: [absent.pem]\n");
    let _sources = Sources::in_dir(dir.path());

    let error = AvisoClient::builder_from_file().unwrap_err();

    assert!(error.to_string().contains("absent.pem"), "got {error}");
    Ok(())
}
