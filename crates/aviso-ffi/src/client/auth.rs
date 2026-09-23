// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Credential setters on the client-builder handle.
//!
//! Three ways to give a builder a credential. `bearer_auth` and `basic_auth`
//! name one the caller holds; `discover_auth` finds one in the environment or
//! on disk. A found credential is refused for a plain http address that is
//! not loopback; a named one is not, because naming it is choosing where it
//! goes. Every failure is remembered on the builder and reported at build.

use std::ffi::c_char;
use std::sync::Arc;

use aviso::auth::{Basic, Bearer};
use aviso::resolve::CodeAuth;

use crate::client::{AvisoClientBuilder, cstr_opt};
use crate::error;
use crate::guard;

/// Sets HTTP Basic credentials on the builder. A null or non-UTF-8 argument, or
/// a credential-construction failure, is remembered and reported at build time.
///
/// # Safety
///
/// `builder` must be a live builder handle from `aviso_client_builder_new`.
/// `username` and `password`, when non-null, must be NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_basic_auth(
    builder: *mut AvisoClientBuilder,
    username: *const c_char,
    password: *const c_char,
) {
    guard((), || {
        // SAFETY: the contract above requires `builder` to be a live handle
        // from `aviso_client_builder_new`; `as_mut` yields `None` for null.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            return;
        };
        if builder.error.is_some() {
            return;
        }
        // SAFETY: the contract above requires each of `username` and
        // `password`, when non-null, to be a NUL-terminated C string.
        let (Some(user), Some(pass)) =
            (unsafe { cstr_opt(username) }, unsafe { cstr_opt(password) })
        else {
            builder.error = Some(error::invalid_input(
                "basic_auth username and password must be non-null and valid UTF-8",
            ));
            return;
        };
        match Basic::new(user, pass) {
            Ok(basic) => {
                let provider: Arc<dyn aviso::auth::AuthProvider> = Arc::new(basic);
                builder.inputs.auth = Some(CodeAuth::Named(provider.clone()));
                builder.apply(|b| b.auth(provider));
            }
            Err(err) => builder.error = Some(error::map_error(&err)),
        }
    });
}

/// Sets a Bearer token on the builder. A null, non-UTF-8 or empty token is
/// remembered and reported at build time.
///
/// This names the credential explicitly, so it is sent to whatever address
/// the builder was given, plain http included. Use
/// `aviso_client_builder_discover_auth` when the token is supplied by the
/// environment or a file rather than by the caller.
///
/// # Safety
///
/// `builder` must be a live builder handle from `aviso_client_builder_new`.
/// `token`, when non-null, must be a NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_bearer_auth(
    builder: *mut AvisoClientBuilder,
    token: *const c_char,
) {
    guard((), || {
        // SAFETY: the contract above requires `builder` to be a live handle
        // from `aviso_client_builder_new`; `as_mut` yields `None` for null.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            return;
        };
        if builder.error.is_some() {
            return;
        }
        // SAFETY: the contract above requires `token`, when non-null, to be a
        // NUL-terminated C string.
        let Some(token) = (unsafe { cstr_opt(token) }) else {
            builder.error = Some(error::invalid_input(
                "bearer_auth token must be non-null and valid UTF-8",
            ));
            return;
        };
        match Bearer::new(token) {
            Ok(bearer) => {
                let provider: Arc<dyn aviso::auth::AuthProvider> = Arc::new(bearer);
                builder.inputs.auth = Some(CodeAuth::Named(provider.clone()));
                builder.apply(|b| b.auth(provider));
            }
            Err(err) => builder.error = Some(error::map_error(&err)),
        }
    });
}

/// Looks for a credential and uses it if one is found.
///
/// The search order is the environment, then the `auth:` block of the config
/// file, then the credentials file, which is the same order the `aviso` binary
/// uses. Finding nothing changes nothing: a credential set earlier with
/// `aviso_client_builder_bearer_auth` or `aviso_client_builder_basic_auth`
/// stays, and a builder with none stays anonymous. Finding a source that
/// cannot be used is remembered and reported at build time.
///
/// A credential found this way is not sent to a plain http address unless it
/// is loopback; that too is reported at build time. When the caller holds the
/// credential, `aviso_client_builder_bearer_auth` or
/// `aviso_client_builder_basic_auth` name it instead, and a named credential
/// goes to whatever address the builder was given.
///
/// # Safety
///
/// `builder` must be a live builder handle from `aviso_client_builder_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aviso_client_builder_discover_auth(builder: *mut AvisoClientBuilder) {
    guard((), || {
        // SAFETY: the contract above requires `builder` to be a live handle
        // from `aviso_client_builder_new`; `as_mut` yields `None` for null.
        let Some(builder) = (unsafe { builder.as_mut() }) else {
            return;
        };
        if builder.error.is_some() {
            return;
        }
        // The address check is not applied here: the caller may still change
        // the address, and build checks the final one. Attaching through
        // found_auth keeps the record that the credential was found.
        builder.searched = true;
        match aviso::auth::discover_with(&builder.paths) {
            Ok(Some(found)) => {
                // A found credential replaces a named one; finding nothing
                // leaves the named one in place, so the record stays too.
                builder.inputs.auth = None;
                builder.apply(|b| b.found_auth(found));
            }
            Ok(None) => {}
            Err(err) => builder.error = Some(error::map_error(&err)),
        }
    });
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
pub(crate) mod tests {
    use std::ffi::CString;

    use super::*;
    use crate::client::{
        AvisoClient, aviso_client_builder_build, aviso_client_builder_from_file,
        aviso_client_builder_from_file_at, aviso_client_builder_new, aviso_client_free,
    };
    use crate::outcome::{aviso_outcome_free, aviso_outcome_take_client};
    use crate::runtime;

    #[test]
    fn bearer_auth_sends_the_token_and_is_allowed_on_plain_http() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        let url = CString::new("http://aviso.example.org").expect("cstring");
        let token = CString::new("explicit-token").expect("cstring");
        let mut builder = unsafe { aviso_client_builder_new(url.as_ptr()) };

        unsafe { aviso_client_builder_bearer_auth(builder, token.as_ptr()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        let client = unsafe { aviso_outcome_take_client(outcome) };
        unsafe { aviso_outcome_free(outcome) };

        // Naming the credential is choosing where it goes, so plain http is
        // not refused the way a discovered credential would be.
        assert!(
            !client.is_null(),
            "an explicit bearer must build on plain http"
        );
        let inner = unsafe { &*client };
        let header = runtime()
            .block_on(inner.inner.auth().expect("auth set").authorization_header())
            .expect("header");
        assert_eq!(header, "Bearer explicit-token");
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn bearer_auth_with_an_empty_token_fails_at_build() {
        let url = CString::new("https://aviso.example.org").expect("cstring");
        let token = CString::new("").expect("cstring");
        let mut builder = unsafe { aviso_client_builder_new(url.as_ptr()) };

        unsafe { aviso_client_builder_bearer_auth(builder, token.as_ptr()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };

        let client = unsafe { aviso_outcome_take_client(outcome) };
        assert!(client.is_null(), "an empty token must be reported");
        unsafe { aviso_outcome_free(outcome) };
    }

    #[test]
    fn bearer_auth_with_a_null_token_fails_at_build() {
        let url = CString::new("https://aviso.example.org").expect("cstring");
        let mut builder = unsafe { aviso_client_builder_new(url.as_ptr()) };

        unsafe { aviso_client_builder_bearer_auth(builder, std::ptr::null()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };

        assert!(unsafe { aviso_outcome_take_client(outcome) }.is_null());
        unsafe { aviso_outcome_free(outcome) };
    }

    #[test]
    fn discover_auth_on_a_null_builder_is_a_no_op() {
        unsafe { aviso_client_builder_discover_auth(std::ptr::null_mut()) };
    }

    /// Serialises the discovery tests: they set process-wide environment
    /// variables, which the test harness runs in parallel by default.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Points every credential source at `dir` and restores the previous
    /// values when dropped, so a variable set on the developer's machine does
    /// not change what these tests exercise.
    pub(crate) struct CredentialEnv {
        _guard: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl CredentialEnv {
        pub(crate) fn pointing_at(
            dir: &std::path::Path,
            credentials: Option<&std::path::Path>,
        ) -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let names = [
                "AVISO_TOKEN",
                "AVISO_USERNAME",
                "AVISO_PASSWORD",
                "AVISO_BASE_URL",
                "AVISO_CLIENT_CONFIG_FILE",
                "AVISO_CREDENTIALS_FILE",
            ];
            let saved = names.iter().map(|k| (*k, std::env::var_os(k))).collect();
            // SAFETY: ENV_LOCK is held, so no other test in this binary is
            // reading or writing these variables while they are changed.
            unsafe {
                for name in [
                    "AVISO_TOKEN",
                    "AVISO_USERNAME",
                    "AVISO_PASSWORD",
                    "AVISO_BASE_URL",
                ] {
                    std::env::remove_var(name);
                }
                std::env::set_var("AVISO_CLIENT_CONFIG_FILE", dir.join("absent-config.yaml"));
                match credentials {
                    Some(path) => std::env::set_var("AVISO_CREDENTIALS_FILE", path),
                    None => {
                        std::env::set_var(
                            "AVISO_CREDENTIALS_FILE",
                            dir.join("absent-credentials.yaml"),
                        );
                    }
                }
            }
            Self {
                _guard: guard,
                saved,
            }
        }
    }

    impl Drop for CredentialEnv {
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

    fn build_with_discovery(url: &str) -> *mut AvisoClient {
        let raw_url = CString::new(url).expect("cstring");
        let mut builder = unsafe { aviso_client_builder_new(raw_url.as_ptr()) };
        unsafe { aviso_client_builder_discover_auth(builder) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        let client = unsafe { aviso_outcome_take_client(outcome) };
        unsafe { aviso_outcome_free(outcome) };
        client
    }

    #[test]
    fn discover_auth_refuses_a_found_credential_for_a_plaintext_address() {
        let dir = tempfile::tempdir().expect("tempdir");
        let credentials = dir.path().join("credentials.yaml");
        std::fs::write(&credentials, "bearer:\n  token: sekrit\n").expect("write");
        let _env = CredentialEnv::pointing_at(dir.path(), Some(&credentials));

        let client = build_with_discovery("http://aviso.example.org");

        assert!(
            client.is_null(),
            "a discovered credential must not reach a plaintext address"
        );
    }

    #[test]
    fn discover_auth_uses_a_found_credential_for_a_loopback_address() {
        let dir = tempfile::tempdir().expect("tempdir");
        let credentials = dir.path().join("credentials.yaml");
        std::fs::write(&credentials, "bearer:\n  token: sekrit\n").expect("write");
        let _env = CredentialEnv::pointing_at(dir.path(), Some(&credentials));

        let client = build_with_discovery("http://127.0.0.1:8000");

        assert!(!client.is_null(), "loopback should accept the credential");
        unsafe { aviso_client_free(client) };
    }

    #[cfg(unix)]
    #[test]
    fn a_valid_token_wins_before_the_basic_pair_is_read() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        // SAFETY: CredentialEnv holds ENV_LOCK, and restores these on drop.
        unsafe {
            std::env::set_var("AVISO_TOKEN", "valid-token");
            std::env::set_var("AVISO_USERNAME", std::ffi::OsStr::from_bytes(&[0xff, 0xfe]));
        }

        let found = aviso::auth::env_provider()
            .expect("a valid token must not be rejected by a lower-priority variable")
            .expect("token present");

        let header = runtime()
            .block_on(found.authorization_header())
            .expect("header");

        assert_eq!(header, "Bearer valid-token");
    }

    #[test]
    fn from_file_reads_the_address_and_finds_the_credential() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("config.yaml"),
            "base_url: https://aviso.example.org\nauth:\n  bearer_token: from-file\n",
        )
        .expect("write");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        // SAFETY: CredentialEnv holds ENV_LOCK and restores this on drop.
        unsafe {
            std::env::set_var("AVISO_CLIENT_CONFIG_FILE", dir.path().join("config.yaml"));
        }

        let mut builder = aviso_client_builder_from_file();
        assert!(!builder.is_null());
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        let client = unsafe { aviso_outcome_take_client(outcome) };
        unsafe { aviso_outcome_free(outcome) };

        assert!(!client.is_null(), "the file supplied everything needed");
        let inner = unsafe { &*client };
        assert_eq!(
            inner.inner.base_url().as_str(),
            "https://aviso.example.org/"
        );
        let header = runtime()
            .block_on(inner.inner.auth().expect("auth set").authorization_header())
            .expect("header");
        assert_eq!(header, "Bearer from-file");
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn from_file_at_a_missing_path_fails_at_build() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        let path =
            CString::new(dir.path().join("absent.yaml").to_str().expect("utf8")).expect("cstring");

        let mut builder = unsafe { aviso_client_builder_from_file_at(path.as_ptr()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };

        assert!(unsafe { aviso_outcome_take_client(outcome) }.is_null());
        unsafe { aviso_outcome_free(outcome) };
    }

    #[test]
    fn from_file_at_a_null_path_fails_at_build() {
        let mut builder = unsafe { aviso_client_builder_from_file_at(std::ptr::null()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };

        assert!(unsafe { aviso_outcome_take_client(outcome) }.is_null());
        unsafe { aviso_outcome_free(outcome) };
    }

    #[test]
    fn a_setter_after_from_file_replaces_the_file_value() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("config.yaml"),
            "base_url: https://aviso.example.org\nauth:\n  bearer_token: from-file\n",
        )
        .expect("write");
        let _env = CredentialEnv::pointing_at(dir.path(), None);
        // SAFETY: as above.
        unsafe {
            std::env::set_var("AVISO_CLIENT_CONFIG_FILE", dir.path().join("config.yaml"));
        }
        let token = CString::new("from-code").expect("cstring");

        let mut builder = aviso_client_builder_from_file();
        unsafe { aviso_client_builder_bearer_auth(builder, token.as_ptr()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };
        let client = unsafe { aviso_outcome_take_client(outcome) };
        unsafe { aviso_outcome_free(outcome) };

        let inner = unsafe { &*client };
        let header = runtime()
            .block_on(inner.inner.auth().expect("auth set").authorization_header())
            .expect("header");
        assert_eq!(header, "Bearer from-code");
        unsafe { aviso_client_free(client) };
    }

    #[test]
    fn a_discovered_credential_stays_refused_after_the_address_is_changed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let credentials = dir.path().join("credentials.yaml");
        std::fs::write(&credentials, "bearer:\n  token: sekrit\n").expect("write");
        let _env = CredentialEnv::pointing_at(dir.path(), Some(&credentials));
        let safe = CString::new("https://safe.example.org").expect("cstring");
        let public = CString::new("http://public.example.org").expect("cstring");

        // Discover while the address is safe, then switch it. The credential
        // was found, not named, so the switch must not launder it.
        let mut builder = unsafe { aviso_client_builder_new(safe.as_ptr()) };
        unsafe { aviso_client_builder_discover_auth(builder) };
        unsafe { crate::client::aviso_client_builder_base_url(builder, public.as_ptr()) };
        let outcome = unsafe { aviso_client_builder_build(&raw mut builder) };

        assert!(
            unsafe { aviso_outcome_take_client(outcome) }.is_null(),
            "the credential was found; a later plaintext address must be refused"
        );
        unsafe { aviso_outcome_free(outcome) };
    }

    #[test]
    fn discover_auth_with_no_credential_anywhere_still_builds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _env = CredentialEnv::pointing_at(dir.path(), None);

        let client = build_with_discovery("http://aviso.example.org");

        assert!(!client.is_null(), "no credential means nothing to refuse");
        unsafe { aviso_client_free(client) };
    }
}
