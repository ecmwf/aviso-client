//! `--from <VALUE>` parser per Amendment H.
//!
//! Accepts seven input forms, tried in this order:
//!
//! 1. Pure-digit `u64` -> [`ResumeStart::AfterSequence`]. Always
//!    wins for digit-only input; compact `YYYYMMDD` is therefore
//!    NOT supported as a date (operators write dates with dashes).
//! 2. `YYYY-MM-DD` -> midnight UTC.
//! 3. `YYYY-MM-DD HH:MM` (space separator) -> UTC.
//! 4. `YYYY-MM-DD HH:MM:SS` (space separator) -> UTC.
//! 5. `YYYY-MM-DDTHH:MM:SS` (T separator, no Z) -> UTC.
//! 6. `YYYY-MM-DDTHH:MM:SSZ` (T separator, explicit Z) -> UTC.
//! 7. `YYYY-MM-DDTHH:MM:SS.ffffffZ` (pyaviso-strict) -> UTC.
//!
//! All six date forms are normalised to the wire format
//! `YYYY-MM-DDTHH:MM:SS.ffffffZ` with EXACTLY six fractional
//! digits (microsecond precision; matches the pyaviso server
//! contract). Input forms with fewer fractional digits are
//! zero-padded; nine-digit input is truncated. The leap-second
//! microsecond clamp coerces chrono's leap-second representation
//! (nanosecond >= 1_000_000_000) to 999_999 microseconds to keep
//! the wire format six-digit-exact.
//!
//! Shell quoting: the three space-separator forms MUST be passed
//! as a single argv item:
//!     aviso listen --from "2024-01-15 14:30"
//!     aviso listen --from "2024-01-15 14:30:00"
//! T-separator forms need no quoting. The docs/src/usage/cli.md
//! '--from value formats' section explains this for operators.

use std::fmt;

use aviso::watch::ResumeStart;
use chrono::{NaiveDate, NaiveDateTime, Timelike};

use crate::exit::usage_error;

const DATE_ONLY_FORMAT: &str = "%Y-%m-%d";
const TIME_FORMATS: &[&str] = &[
    "%Y-%m-%d %H:%M",
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%dT%H:%M:%SZ",
    "%Y-%m-%dT%H:%M:%S%.fZ",
];

const ACCEPTED_FORMS: &str = "accepted forms: pure-digit sequence id (e.g. 42); \
                              YYYY-MM-DD; \
                              \"YYYY-MM-DD HH:MM\" (quotes required); \
                              \"YYYY-MM-DD HH:MM:SS\" (quotes required); \
                              YYYY-MM-DDTHH:MM:SS; \
                              YYYY-MM-DDTHH:MM:SSZ; \
                              YYYY-MM-DDTHH:MM:SS.ffffffZ";

/// Parses `value` into a [`ResumeStart`].
///
/// Returns a usage error tagged via [`crate::exit::usage_error`]
/// when `value` matches none of the seven accepted forms; the
/// error message names the value and lists the accepted forms.
pub(crate) fn parse(value: &str) -> anyhow::Result<ResumeStart> {
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        if let Ok(seq) = value.parse::<u64>() {
            return Ok(ResumeStart::AfterSequence(seq));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, DATE_ONLY_FORMAT) {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| usage_error("invalid date: midnight construction failed"))?;
        return Ok(ResumeStart::Date(format_utc_six_micros(dt)));
    }
    for fmt in TIME_FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(value, fmt) {
            return Ok(ResumeStart::Date(format_utc_six_micros(dt)));
        }
    }
    Err(usage_error(format!(
        "parameter parse: --from value `{value}` did not match any accepted form. {ACCEPTED_FORMS}"
    )))
}

/// Returns the wire-format string `YYYY-MM-DDTHH:MM:SS.ffffffZ`.
///
/// The microsecond clamp guards against the chrono leap-second
/// representation where `nanosecond()` can be >= 1_000_000_000.
fn format_utc_six_micros(dt: NaiveDateTime) -> String {
    let micros = (u64::from(dt.nanosecond()) / 1000).min(999_999);
    Stamped {
        date: dt.format("%Y-%m-%dT%H:%M:%S"),
        micros,
    }
    .to_string()
}

struct Stamped<F: fmt::Display> {
    date: F,
    micros: u64,
}

impl<F: fmt::Display> fmt::Display for Stamped<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:06}Z", self.date, self.micros)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect/panic on parse success is the expected diagnostic"
)]
mod tests {
    use super::*;

    fn parse_ok(s: &str) -> ResumeStart {
        parse(s).expect("input is a recognised form")
    }

    fn parse_date(s: &str) -> String {
        match parse_ok(s) {
            ResumeStart::Date(d) => d,
            other => panic!("expected Date variant, got {other:?}"),
        }
    }

    fn parse_id(s: &str) -> u64 {
        match parse_ok(s) {
            ResumeStart::AfterSequence(n) => n,
            other => panic!("expected AfterSequence variant, got {other:?}"),
        }
    }

    #[test]
    fn pure_digit_routes_to_after_sequence() {
        assert_eq!(parse_id("0"), 0);
        assert_eq!(parse_id("42"), 42);
        assert_eq!(parse_id("18446744073709551615"), u64::MAX);
    }

    #[test]
    fn compact_date_form_treated_as_sequence_id() {
        assert_eq!(parse_id("20240115"), 20_240_115);
    }

    #[test]
    fn iso_date_only_normalises_to_midnight() {
        assert_eq!(parse_date("2024-01-15"), "2024-01-15T00:00:00.000000Z");
    }

    #[test]
    fn iso_date_space_hours_minutes() {
        assert_eq!(
            parse_date("2024-01-15 14:30"),
            "2024-01-15T14:30:00.000000Z"
        );
    }

    #[test]
    fn iso_date_space_full_time() {
        assert_eq!(
            parse_date("2024-01-15 14:30:45"),
            "2024-01-15T14:30:45.000000Z"
        );
    }

    #[test]
    fn iso_t_separator_no_z() {
        assert_eq!(
            parse_date("2024-01-15T14:30:45"),
            "2024-01-15T14:30:45.000000Z"
        );
    }

    #[test]
    fn iso_t_separator_with_z() {
        assert_eq!(
            parse_date("2024-01-15T14:30:45Z"),
            "2024-01-15T14:30:45.000000Z"
        );
    }

    #[test]
    fn pyaviso_strict_six_digit_fractional() {
        let s = "2024-01-15T14:30:45.123456Z";
        assert_eq!(parse_date(s), s);
    }

    #[test]
    fn one_fractional_digit_pads_to_six() {
        assert_eq!(
            parse_date("2024-01-15T14:30:45.1Z"),
            "2024-01-15T14:30:45.100000Z"
        );
    }

    #[test]
    fn nine_fractional_digits_truncate_to_six() {
        assert_eq!(
            parse_date("2024-01-15T14:30:45.123456789Z"),
            "2024-01-15T14:30:45.123456Z"
        );
    }

    #[test]
    fn leading_plus_sign_not_treated_as_pure_digit() {
        let err = parse("+42").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("+42"), "{msg}");
    }

    #[test]
    fn whitespace_padded_digit_not_treated_as_pure_digit() {
        let err = parse(" 42").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("42") || msg.contains("accepted forms"),
            "{msg}"
        );
    }

    #[test]
    fn unrecognised_form_errors_with_accepted_list() {
        let err = parse("not-a-date").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not-a-date"), "{msg}");
        assert!(msg.contains("accepted forms"), "{msg}");
    }
}
