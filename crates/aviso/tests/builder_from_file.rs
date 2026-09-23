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

#[path = "common/env.rs"]
mod env;

use aviso::{AvisoClient, AvisoClientBuilder, ClientError};
use env::{Sources, write_config};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

type TestResult = Result<(), Box<dyn std::error::Error>>;

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

/// Self-signed X.509 CA certificate, the same one the builder's own TLS
/// tests pin. Committed verbatim so this test is hermetic: no openssl at test
/// time, no network. Expires in year 2126.
const TEST_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----\n\
MIIDDTCCAfWgAwIBAgIUOoEsjJSbNYUFzrZXLulyRChR/XEwDQYJKoZIhvcNAQEL\n\
BQAwFTETMBEGA1UEAwwKYXZpc28tdGVzdDAgFw0yNjA1MjExMDU2MTJaGA8yMTI2\n\
MDQyNzEwNTYxMlowFTETMBEGA1UEAwwKYXZpc28tdGVzdDCCASIwDQYJKoZIhvcN\n\
AQEBBQADggEPADCCAQoCggEBAKvtdr6hpcYQ5R7uHt42S95WQqJn/mm6nJxNyM51\n\
4ELO2MZ7X9Vgvy2aVPHsqDV5vHGzZF0F7F+FLA664HAsPnaaghjBnKSW7s4arUb8\n\
4k0RHUi8sivBxYqr5uGbp8uCcas29icFyznaBWELdPmfUFOhhq/BceSmucCoNg0J\n\
pUxsjqRKtfXpWFI4bpaEmKkNneSYneCqkyWBzy+1DxkYE/yY6vkQqmSgb9gjqq1o\n\
WPPyJSw0yyC/jKTp9L0Nz6l7Tn2gdEHDZ9j1nsFy9DD2ZNQ9qlY8fg497gXoa1Mg\n\
Unxhv9usMD6EWWA8yezRxVMcTOEWT9miGEt+Tj6iGLCtXfcCAwEAAaNTMFEwHQYD\n\
VR0OBBYEFGJb4ns++TufwOE+Cbb0VqZMrO7xMB8GA1UdIwQYMBaAFGJb4ns++Tuf\n\
wOE+Cbb0VqZMrO7xMA8GA1UdEwEB/wQFMAMBAf8wDQYJKoZIhvcNAQELBQADggEB\n\
AJIsFiiJtf425jlvJxXBsYl8AyiQopvs04K1JpfGpIOsKQxKnOZZzSfUrObQAvjr\n\
IMZEksfPfwOJN4LtPjqzFEO3TqDbWq7bfbzd+pPRh36VceznesuDnBA+z1vNKKH+\n\
8naFx24zL9itWLt9Is/6AFbRfbdYsDExpisLhr4XIQblGPFneq4Bkh9l7szKuMts\n\
WH7j++yZ8PoisM0X0wPuCykZiIXTpdzd3tOkz2KYR7sgvoSugQCN+aYPns2DnXj7\n\
++9qepJtLMoAvtOkutza7a0JuMTkKbnOCiyZELeQq6hHpJuoI2T5lugdanmWkUIF\n\
62aTjKqXhHyepRlFSTwwEAk=\n\
-----END CERTIFICATE-----\n";

#[test]
fn timeouts_and_certificates_from_the_file_reach_the_builder() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("ca.pem"), TEST_CA_PEM)?;
    write_config(
        dir.path(),
        "base_url: https://aviso.example.org\ntimeout: 7s\nheartbeat_interval: 11s\n\
         tls:\n  ca_bundle: [ca.pem]\n",
    );
    let _sources = Sources::in_dir(dir.path());

    let builder = AvisoClient::builder_from_file()?;
    let rendered = format!("{builder:?}");

    assert!(rendered.contains("timeout: Some(7s)"), "got {rendered}");
    assert!(
        rendered.contains("heartbeat_interval: Some(11s)"),
        "got {rendered}"
    );
    assert!(
        rendered.contains("extra_root_certs_count: 1"),
        "got {rendered}"
    );
    Ok(())
}

#[test]
fn a_file_that_is_not_a_certificate_is_reported() -> TestResult {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("ca.pem"), "not a certificate")?;
    write_config(dir.path(), "tls:\n  ca_bundle: [ca.pem]\n");
    let _sources = Sources::in_dir(dir.path());

    let error = AvisoClient::builder_from_file().unwrap_err();

    assert!(error.to_string().contains("ca.pem"), "got {error}");
    assert!(matches!(error, ClientError::Config(_)), "got {error:?}");
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

#[test]
fn found_auth_is_the_marked_path_and_auth_is_the_named_one() -> TestResult {
    // The distinction every discovery caller relies on: attaching through
    // found_auth keeps the plaintext rule, attaching through auth lifts it.
    let dir = tempfile::tempdir()?;
    write_config(dir.path(), "auth:\n  bearer_token: from-file\n");
    let _sources = Sources::in_dir(dir.path());
    let mut paths = aviso::auth::DiscoveryPaths::from_env();
    paths.config_file = Some(dir.path().join("config.yaml"));

    let found = aviso::auth::discover_with(&paths)?.expect("credential");
    let refused = AvisoClient::builder()
        .base_url("http://public.example.org")
        .found_auth(found)
        .build();
    assert!(
        matches!(refused, Err(ClientError::Auth(_))),
        "got {refused:?}"
    );

    let found = aviso::auth::discover_with(&paths)?.expect("credential");
    AvisoClient::builder()
        .base_url("http://public.example.org")
        .auth(found.into_provider())
        .build()?;
    Ok(())
}
