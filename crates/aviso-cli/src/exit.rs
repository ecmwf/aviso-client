// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Exit-code taxonomy for the `aviso` binary.
//!
//! Three semantic exit codes plus the hard-SIGINT escape hatch:
//!
//! - `0` success.
//! - `1` runtime error (HTTP 4xx / 5xx, network failure, file I/O,
//!   listener task errored or panicked).
//! - `2` usage error (missing required flag, invalid argument value,
//!   destructive admin command without `--yes`, no listeners resolved
//!   for `aviso listen`).
//! - `130` second-SIGINT hard exit (`128 + SIGINT`).
//!
//! The library uses anyhow internally; the binary's exit-code mapping
//! is the single seam where the anyhow chain becomes a numeric exit
//! status. Subcommand handlers raise typed usage errors via
//! [`usage_error`]; the top-level error formatter sees those and
//! routes them to exit `2`. Everything else flows through exit `1`.

/// Sentinel exit-code values used by [`exit_code_for_anyhow`].
///
/// Kept as plain integer constants because tokio's process-exit
/// path (and `std::process::exit`) takes `i32` directly; introducing
/// a typed enum here would only add an `into()` at every call site.
pub(crate) const SUCCESS: i32 = 0;
pub(crate) const RUNTIME_ERROR: i32 = 1;
pub(crate) const USAGE_ERROR: i32 = 2;
pub(crate) const HARD_SIGINT: i32 = 130;

/// Tag-only marker placed in an `anyhow::Error`'s context chain to
/// signal "this is a usage error; exit code 2". The top-level
/// formatter inspects the chain for this tag via
/// [`anyhow::Error::is`] / [`anyhow::Error::downcast_ref`].
///
/// Kept a unit struct (no payload) because the user-visible message
/// already lives on the surrounding anyhow context. The tag is the
/// signal; the context is the message.
#[derive(Debug)]
pub(crate) struct UsageErrorTag;

impl std::fmt::Display for UsageErrorTag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("usage error")
    }
}

impl std::error::Error for UsageErrorTag {}

/// Wraps `message` into an `anyhow::Error` tagged as a usage error.
/// The top-level formatter sees the tag and returns exit code 2.
pub(crate) fn usage_error(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(UsageErrorTag).context(message.into())
}

/// Returns the exit code for `err`. Usage errors (tagged via
/// [`UsageErrorTag`]) map to `2`; everything else maps to `1`.
pub(crate) fn exit_code_for_anyhow(err: &anyhow::Error) -> i32 {
    if err
        .chain()
        .any(<dyn std::error::Error + 'static>::is::<UsageErrorTag>)
    {
        USAGE_ERROR
    } else {
        RUNTIME_ERROR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_error_tag_routes_to_exit_2() {
        let err = usage_error("missing --yes for destructive admin command");
        assert_eq!(exit_code_for_anyhow(&err), USAGE_ERROR);
    }

    #[test]
    fn untagged_error_routes_to_exit_1() {
        let err: anyhow::Error = anyhow::anyhow!("connection refused: tcp transport failed");
        assert_eq!(exit_code_for_anyhow(&err), RUNTIME_ERROR);
    }

    #[test]
    fn usage_error_tag_preserved_through_context_chain() {
        let err = usage_error("invalid --from value")
            .context("parsing --from <VALUE>")
            .context("loading listener");
        assert_eq!(exit_code_for_anyhow(&err), USAGE_ERROR);
    }
}
