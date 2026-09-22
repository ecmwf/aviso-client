// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Where a discovered credential may be sent.
//!
//! A credential the caller never named is treated more carefully than one
//! written into the call. The rule here is the only thing standing between a
//! mistyped host and a token sent in the clear.

use crate::ClientError;
use crate::auth::Discovered;

/// Applies the address rule to an already-completed search.
///
/// Separated so the rule can be tested without a process-wide environment.
pub(super) fn refuse_public_plaintext(
    found: Option<Discovered>,
    base_url: &str,
) -> crate::Result<Option<Discovered>> {
    let Some(found) = found else {
        return Ok(None);
    };
    if !is_public_plaintext(base_url) {
        return Ok(Some(found));
    }
    Err(ClientError::Auth(format!(
        "refusing to send the credential from the {} to {}, which is not \
         https and not a loopback address. Use an https address, or pass the \
         credential explicitly if you intend to send it in the clear.",
        found.source(),
        url_without_userinfo(base_url)
    )))
}

/// The address with any `user:password@` stripped, for use in messages.
///
/// Public so callers that report a refused address can do so without
/// repeating a secret that happened to be in the URL.
///
/// A base URL may carry userinfo, so repeating it verbatim in an error would
/// publish the very kind of secret this module exists to protect. An address
/// that does not parse is reported as a placeholder rather than echoed.
#[must_use]
pub fn url_without_userinfo(base_url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(base_url) else {
        return "<unparseable url>".to_string();
    };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return base_url.to_string();
    }
    if parsed.set_username("").is_err() || parsed.set_password(None).is_err() {
        return "<unparseable url>".to_string();
    }
    parsed.to_string()
}

/// True only for an address that parses, is not `https`, and is not loopback:
/// the one case where a found credential is refused.
///
/// An address that does not parse is not a plaintext address; it is invalid
/// input, and the client builder reports that as a Config error. Returning
/// `false` for it keeps that error intact rather than replacing it with a
/// refusal the caller cannot act on.
#[must_use]
pub fn is_public_plaintext(base_url: &str) -> bool {
    url::Url::parse(base_url).is_ok() && !url_keeps_credentials_private(base_url)
}

/// True when a credential may travel to this address.
///
/// `https` is protected in transit. A loopback address never leaves the
/// machine, so plaintext is fine there and local development keeps working.
/// Anything else, including `http://aviso.example.org`, is refused. An address
/// that does not parse is also `false`; callers that need to tell the two
/// apart parse first, as [`refuse_public_plaintext`] does.
#[must_use]
pub fn url_keeps_credentials_private(base_url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(base_url) else {
        return false;
    };
    if parsed.scheme().eq_ignore_ascii_case("https") {
        return true;
    }
    match parsed.host() {
        Some(url::Host::Domain(name)) => {
            name.eq_ignore_ascii_case("localhost")
                || name.to_ascii_lowercase().ends_with(".localhost")
        }
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on fixture setup are the expected diagnostics"
)]
mod tests {
    use super::*;
    use crate::auth::CredentialSource;

    #[test]
    fn https_and_loopback_addresses_keep_a_credential_private() {
        for url in [
            "https://aviso.example.org",
            "https://aviso.example.org:8443/path",
            "http://localhost:8000",
            "http://127.0.0.1:8000",
            "http://[::1]:8000",
            "http://aviso.localhost:8000",
        ] {
            assert!(
                url_keeps_credentials_private(url),
                "{url} should be allowed"
            );
        }
    }

    #[test]
    fn the_refusal_does_not_repeat_url_userinfo() {
        let found = Some(Discovered::for_test(CredentialSource::Environment));

        let error =
            refuse_public_plaintext(found, "http://alice:hunter2@aviso.example.org").unwrap_err();
        let message = error.to_string();

        assert!(
            !message.contains("hunter2"),
            "the password must not appear: {message}"
        );
        assert!(
            !message.contains("alice"),
            "the username must not appear: {message}"
        );
        assert!(message.contains("aviso.example.org"), "got {message}");
    }

    #[test]
    fn an_unparseable_address_is_left_for_the_builder_to_reject() {
        let found = Some(Discovered::for_test(CredentialSource::Environment));

        // Refusing here would turn the builder's "invalid base_url" Config
        // error into an auth refusal, so the credential passes through.
        assert!(
            refuse_public_plaintext(found, "not a url")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn an_unparseable_address_is_not_echoed_in_messages() {
        assert_eq!(url_without_userinfo("not a url"), "<unparseable url>");
    }

    #[test]
    fn an_address_without_userinfo_is_reported_as_given() {
        assert_eq!(
            url_without_userinfo("http://aviso.example.org"),
            "http://aviso.example.org"
        );
    }

    #[test]
    fn only_a_parseable_remote_plaintext_address_is_public_plaintext() {
        assert!(is_public_plaintext("http://aviso.example.org"));
        assert!(!is_public_plaintext("https://aviso.example.org"));
        assert!(!is_public_plaintext("http://127.0.0.1:8000"));
        assert!(!is_public_plaintext("not a url"));
    }

    #[test]
    fn plaintext_remote_addresses_are_not_private() {
        for url in [
            "http://aviso.example.org",
            "http://10.0.0.5:8000",
            "http://192.168.1.10",
            "not a url",
        ] {
            assert!(
                !url_keeps_credentials_private(url),
                "{url} should be refused"
            );
        }
    }
}
