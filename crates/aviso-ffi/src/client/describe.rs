// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Building from the environment, and saying what a builder resolved.

use std::ffi::CString;
use std::ptr;

use aviso::resolve::{CodeAuth, CodeInputs, EnvAddress};

use crate::client::{AvisoClientBuilder, builder_from};
use crate::outcome::AvisoOutcome;
use crate::{error, guard, guard_outcome};

/// Creates a client builder with nothing named in code, the way a script on
/// a machine set up for the `aviso` command wants one.
///
/// The address comes from `AVISO_BASE_URL`, then the config file's
/// `base_url`; the credential from `AVISO_TOKEN` or `AVISO_USERNAME` with
/// `AVISO_PASSWORD`, then the file's `auth:` block, then the credentials
/// file; timeouts and TLS settings from the file. Setters called afterwards
/// replace what was found. With no address anywhere, or a found credential
/// headed for a plain http address that is not loopback, the error is
/// reported at build time. `aviso_client_builder_describe` shows what was
/// chosen and from where.
///
/// Returns a builder handle, or null only if an internal panic is trapped.
#[unsafe(no_mangle)]
pub extern "C" fn aviso_client_builder_from_environment() -> *mut AvisoClientBuilder {
    guard(ptr::null_mut(), || {
        Box::into_raw(Box::new(builder_from(
            aviso::AvisoClientBuilder::from_environment(&CodeInputs::default()),
            EnvAddress::Read,
        )))
    })
}

/// Reports the settings a client built from this builder would use and
/// where each came from, one per line, without building. Nothing in the
/// text is a secret: the credential is described by kind and source, and
/// the address has any `user:password@` removed, so it can be logged or
/// pasted into a ticket as it is.
///
/// The builder is not consumed. The outcome carries the text; take it with
/// `aviso_outcome_take_string` and free it with `aviso_string_free`. An
/// error remembered by the builder, or a config file that cannot be read,
/// is returned as the outcome's error instead.
///
/// # Safety
///
/// `builder` must be a live builder handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_describe(
    builder: *const AvisoClientBuilder,
) -> *mut AvisoOutcome {
    guard_outcome(|| {
        // SAFETY: the contract above requires `builder` to be a live handle;
        // `as_ref` yields `None` for null.
        let Some(builder) = (unsafe { builder.as_ref() }) else {
            return AvisoOutcome::error(error::invalid_input("builder must be non-null"))
                .into_raw();
        };
        if let Some(err) = &builder.error {
            return AvisoOutcome::error(err.duplicate()).into_raw();
        }
        // A builder that never searched for a credential and had none named
        // is anonymous; the resolver would otherwise report one it found.
        let mut inputs = builder.inputs.clone();
        if inputs.auth.is_none() && !builder.searched {
            inputs.auth = Some(CodeAuth::Anonymous);
        }
        let resolved = aviso::resolve::resolve(&inputs, &builder.paths, builder.env_address);
        match resolved {
            Ok(resolution) => match CString::new(resolution.settings.to_string()) {
                Ok(text) => AvisoOutcome::text(text),
                Err(_) => {
                    AvisoOutcome::error(error::internal("the report contained an interior NUL"))
                }
            },
            Err(err) => AvisoOutcome::error(error::map_error(&err)),
        }
        .into_raw()
    })
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test code: expect on known-valid inputs is the standard test diagnostic"
)]
#[allow(
    clippy::undocumented_unsafe_blocks,
    reason = "test code: every unsafe block here is a call into the crate's own C ABI from Rust, with the arguments constructed a few lines above"
)]
mod tests {
    use std::ffi::{CStr, CString};

    use super::*;
    use crate::client::auth::aviso_client_builder_bearer_auth;
    use crate::client::auth::tests::CredentialEnv;
    use crate::client::{
        aviso_client_builder_build, aviso_client_builder_free, aviso_client_builder_new,
        aviso_client_free,
    };
    use crate::outcome::{
        aviso_outcome_error, aviso_outcome_free, aviso_outcome_take_client,
        aviso_outcome_take_string, aviso_string_free,
    };

    fn describe(builder: *const AvisoClientBuilder) -> Result<String, String> {
        let outcome = unsafe { aviso_client_builder_describe(builder) };
        let text = unsafe { aviso_outcome_take_string(outcome) };
        let result = if text.is_null() {
            let error = unsafe { aviso_outcome_error(outcome) };
            let message = unsafe { (*error).message };
            Err(unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned())
        } else {
            let owned = unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned();
            unsafe { aviso_string_free(text) };
            Ok(owned)
        };
        unsafe { aviso_outcome_free(outcome) };
        result
    }

    #[test]
    fn from_environment_reads_the_address_from_the_variable() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("config.yaml"),
            "base_url: https://file.example.org\nauth:\n  bearer_token: from-file\n",
        )
        .expect("write config");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        unsafe { std::env::set_var("AVISO_CLIENT_CONFIG_FILE", dir.path().join("config.yaml")) };
        unsafe { std::env::set_var("AVISO_BASE_URL", "https://env.example.org") };

        let mut builder = aviso_client_builder_from_environment();
        assert!(!builder.is_null());
        let report = describe(builder).expect("report");
        assert!(report.contains("https://env.example.org/"), "got: {report}");
        assert!(
            report.contains("environment AVISO_BASE_URL"),
            "got: {report}"
        );
        assert!(report.contains("bearer"), "got: {report}");
        assert!(!report.contains("from-file"), "got: {report}");

        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        let client = unsafe { aviso_outcome_take_client(outcome) };
        assert!(!client.is_null(), "build failed");
        unsafe { aviso_outcome_free(outcome) };
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn describe_reports_a_named_credential_and_leaves_the_builder_usable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        unsafe { std::env::set_var("AVISO_TOKEN", "env-token") };
        let url = CString::new("http://public.example.org").expect("cstring");
        let token = CString::new("named").expect("cstring");
        let mut builder = unsafe { aviso_client_builder_new(url.as_ptr()) };
        unsafe { aviso_client_builder_bearer_auth(builder, token.as_ptr()) };

        let report = describe(builder).expect("report");
        assert!(
            report.contains("http://public.example.org/"),
            "got: {report}"
        );
        assert!(!report.contains("\"named\""), "got: {report}");
        assert!(!report.contains("env-token"), "got: {report}");
        let auth_line = report
            .lines()
            .find(|l| l.starts_with("auth"))
            .expect("auth line");
        assert!(auth_line.contains("bearer"), "got: {auth_line}");
        assert!(auth_line.contains("(code)"), "got: {auth_line}");

        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        let client = unsafe { aviso_outcome_take_client(outcome) };
        assert!(!client.is_null(), "build failed");
        unsafe { aviso_outcome_free(outcome) };
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn describe_calls_an_unsearched_builder_anonymous() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        unsafe { std::env::set_var("AVISO_TOKEN", "env-token") };
        let url = CString::new("https://aviso.example.org").expect("cstring");
        let builder = unsafe { aviso_client_builder_new(url.as_ptr()) };

        let report = describe(builder).expect("report");
        let auth_line = report
            .lines()
            .find(|l| l.starts_with("auth"))
            .expect("auth line");
        assert!(auth_line.contains("anonymous"), "got: {auth_line}");
        unsafe { aviso_client_builder_free(builder) };
    }

    #[test]
    fn describe_reads_the_named_file_not_the_default_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("default.yaml"),
            "base_url: https://default.example.org\n",
        )
        .expect("write default");
        std::fs::write(
            dir.path().join("named.yaml"),
            "base_url: https://named.example.org\n",
        )
        .expect("write named");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        unsafe { std::env::set_var("AVISO_CLIENT_CONFIG_FILE", dir.path().join("default.yaml")) };
        let path = CString::new(dir.path().join("named.yaml").to_string_lossy().into_owned())
            .expect("cstring");

        let builder = unsafe { crate::client::aviso_client_builder_from_file_at(path.as_ptr()) };
        let report = describe(builder).expect("report");
        assert!(
            report.contains("https://named.example.org/"),
            "got: {report}"
        );
        assert!(!report.contains("default.example.org"), "got: {report}");
        unsafe { aviso_client_builder_free(builder) };
    }

    #[test]
    fn a_named_credential_survives_a_search_that_finds_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        let url = CString::new("https://aviso.example.org").expect("cstring");
        let token = CString::new("named").expect("cstring");
        let builder = unsafe { aviso_client_builder_new(url.as_ptr()) };
        unsafe { aviso_client_builder_bearer_auth(builder, token.as_ptr()) };
        unsafe { crate::client::auth::aviso_client_builder_discover_auth(builder) };

        let report = describe(builder).expect("report");
        let auth_line = report
            .lines()
            .find(|l| l.starts_with("auth"))
            .expect("auth line");
        assert!(auth_line.contains("bearer"), "got: {auth_line}");
        assert!(auth_line.contains("(code)"), "got: {auth_line}");
        unsafe { aviso_client_builder_free(builder) };
    }

    #[test]
    fn describe_returns_the_error_a_builder_remembers() {
        let builder = unsafe { aviso_client_builder_new(ptr::null()) };
        let err = describe(builder).expect_err("remembered error");
        assert!(err.contains("base_url"), "got: {err}");
        unsafe { aviso_client_builder_free(builder) };
    }
}
