// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `--from <VALUE>` and `--until <VALUE>` parser per Amendment H.
//!
//! Both flags accept the same seven input forms, tried in this order:
//!
//! 1. Pure-digit `u64` -> a sequence: [`ResumeStart::AfterSequence`] for
//!    `--from` (exclusive) and [`ReplayEnd::Sequence`] for `--until`
//!    (inclusive). Always
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
//! T-separator forms need no quoting. The docs/src/cli/configuration.md
//! '`--from` value formats' section explains this for operators.

use std::fmt;

use aviso::watch::{ReplayEnd, ResumeStart};
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

/// A parsed position: a sequence id or a normalised date.
enum Position {
    Sequence(u64),
    Date(String),
}

/// Parses `value` given to the flag `flag` (`--from` or `--until`).
///
/// Returns a usage error tagged via [`crate::exit::usage_error`]
/// when `value` matches none of the seven accepted forms; the
/// error message names the flag and the value and lists the
/// accepted forms.
fn parse_position(value: &str, flag: &str) -> anyhow::Result<Position> {
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return match value.parse::<u64>() {
            Ok(seq) => Ok(Position::Sequence(seq)),
            Err(_) => Err(usage_error(format!(
                "parameter parse: {flag} value `{value}` is digit-only but does not fit in a 64-bit unsigned integer (max {}). Pass a smaller sequence id, or use one of the date forms. {ACCEPTED_FORMS}",
                u64::MAX
            ))),
        };
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, DATE_ONLY_FORMAT) {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| usage_error("invalid date: midnight construction failed"))?;
        return Ok(Position::Date(format_utc_six_micros(dt)));
    }
    for fmt in TIME_FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(value, fmt) {
            return Ok(Position::Date(format_utc_six_micros(dt)));
        }
    }
    Err(usage_error(format!(
        "parameter parse: {flag} value `{value}` did not match any accepted form. {ACCEPTED_FORMS}"
    )))
}

/// Parses a `--from` value into a [`ResumeStart`]: a sequence id is the
/// last one already seen, so the replay starts after it.
pub(crate) fn parse(value: &str) -> anyhow::Result<ResumeStart> {
    Ok(match parse_position(value, "--from")? {
        Position::Sequence(seq) => ResumeStart::AfterSequence(seq),
        Position::Date(date) => ResumeStart::Date(date),
    })
}

/// Parses an `--until` value into a [`ReplayEnd`]: a sequence id is the
/// last one to deliver, inclusive, and a date ends with the last
/// notification stored at or before it.
pub(crate) fn parse_until(value: &str) -> anyhow::Result<ReplayEnd> {
    Ok(match parse_position(value, "--until")? {
        Position::Sequence(seq) => ReplayEnd::Sequence(seq),
        Position::Date(date) => ReplayEnd::Date(date),
    })
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

    #[test]
    fn digit_only_overflow_reports_out_of_range_not_unrecognised_form() {
        let too_big = "99999999999999999999999999";
        let err = parse(too_big).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(too_big),
            "error should name the input value: {msg}"
        );
        assert!(
            msg.contains("does not fit") && msg.contains("64-bit"),
            "error should report u64 overflow, not generic 'did not match any accepted form': {msg}"
        );
    }

    #[test]
    fn until_digits_are_an_inclusive_sequence_end() {
        assert_eq!(parse_until("20").unwrap(), ReplayEnd::Sequence(20));
    }

    #[test]
    fn until_accepts_the_from_date_forms() {
        assert_eq!(
            parse_until("2026-05-18").unwrap(),
            ReplayEnd::Date("2026-05-18T00:00:00.000000Z".into())
        );
        assert_eq!(
            parse_until("2026-05-18 12:30").unwrap(),
            ReplayEnd::Date("2026-05-18T12:30:00.000000Z".into())
        );
    }

    #[test]
    fn until_errors_name_the_until_flag() {
        let error = parse_until("tomorrow").unwrap_err().to_string();
        assert!(error.contains("--until value `tomorrow`"), "{error}");
        assert!(!error.contains("--from"), "{error}");
    }

    /// Fills a listed date form with a concrete value.
    fn fill(form: &str) -> String {
        form.replacen("YYYY-MM-DD", "2026-05-01", 1)
            .replacen("HH", "14", 1)
            .replacen("MM", "30", 1)
            .replacen("SS", "15", 1)
            .replacen("ffffff", "123456", 1)
    }

    #[test]
    fn every_accepted_form_parses_for_both_flags() {
        let forms: Vec<&str> = ACCEPTED_FORMS
            .trim_start_matches("accepted forms: ")
            .split("; ")
            .collect();
        assert_eq!(forms.len(), 7, "{forms:?}");
        assert_eq!(forms[0], "pure-digit sequence id (e.g. 42)");
        assert!(matches!(parse_ok("42"), ResumeStart::AfterSequence(42)));
        assert!(matches!(
            parse_until("42").unwrap(),
            ReplayEnd::Sequence(42)
        ));
        for form in &forms[1..] {
            let value = fill(
                form.trim_end_matches(" (quotes required)")
                    .trim_matches('"'),
            );
            assert!(matches!(parse_ok(&value), ResumeStart::Date(_)), "{value}");
            assert!(
                matches!(parse_until(&value).unwrap(), ReplayEnd::Date(_)),
                "{value}"
            );
        }
    }
}
