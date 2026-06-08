// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! HTTP `Retry-After` header parser.
//!
//! RFC 9110 section 10.2.3 accepts two forms:
//! - A non-negative decimal integer (delta-seconds): `Retry-After: 120`.
//! - An HTTP-date: `Retry-After: Fri, 31 Dec 1999 23:59:59 GMT`.
//!
//! The parser handles both. Returned values are capped at five minutes
//! per the resolved decision: an honest server might request a long
//! pause, but a misconfigured or hostile one could request hours; the
//! cap bounds worst-case wait time without losing the polite intent.

use std::time::{Duration, SystemTime};

use reqwest::header::HeaderValue;

/// Upper bound on the honoured wait time, regardless of what the
/// header requested. Five minutes is generous enough for any real
/// rate-limited deployment and small enough that a misconfigured
/// server cannot effectively pause a watch indefinitely.
const MAX_HONOURED: Duration = Duration::from_secs(300);

/// Parse an HTTP `Retry-After` header.
///
/// Returns `None` for absent, malformed, in-the-past, or otherwise
/// unparseable values. The caller (the watch supervisor's reconnect
/// loop) falls back to the standard exponential-backoff schedule on
/// `None`.
#[must_use]
pub(crate) fn parse_retry_after(value: Option<&HeaderValue>) -> Option<Duration> {
    let raw = value?.to_str().ok()?.trim();

    // Delta-seconds form: a non-negative decimal integer.
    if let Ok(secs) = raw.parse::<u64>() {
        return Some(Duration::from_secs(secs).min(MAX_HONOURED));
    }

    // HTTP-date form.
    let target = httpdate::parse_http_date(raw).ok()?;
    let now = SystemTime::now();
    let delta = target.duration_since(now).ok()?;
    Some(delta.min(MAX_HONOURED))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: unwrap and expect on well-formed fixtures are the expected diagnostics"
)]
mod tests {
    use std::time::{Duration, SystemTime};

    use reqwest::header::HeaderValue;

    use super::{MAX_HONOURED, parse_retry_after};

    fn header(s: &str) -> HeaderValue {
        HeaderValue::from_str(s).expect("test fixtures are well-formed header values")
    }

    #[test]
    fn delta_seconds_30_returns_30s() {
        let v = header("30");
        assert_eq!(parse_retry_after(Some(&v)), Some(Duration::from_secs(30)));
    }

    #[test]
    fn delta_seconds_zero_returns_zero() {
        let v = header("0");
        assert_eq!(parse_retry_after(Some(&v)), Some(Duration::ZERO));
    }

    #[test]
    fn delta_seconds_huge_caps_at_5min() {
        let v = header("86400");
        assert_eq!(parse_retry_after(Some(&v)), Some(MAX_HONOURED));
    }

    #[test]
    fn delta_seconds_exactly_at_cap_returns_cap() {
        let v = header("300");
        assert_eq!(parse_retry_after(Some(&v)), Some(MAX_HONOURED));
    }

    #[test]
    fn http_date_in_future_returns_correct_delta() {
        let target = SystemTime::now() + Duration::from_secs(60);
        let formatted = httpdate::fmt_http_date(target);
        let v = header(&formatted);
        let parsed = parse_retry_after(Some(&v)).expect("future HTTP-date must parse");
        // Allow a small slack because SystemTime::now() ticks between the
        // formatting call here and the now() call inside the parser.
        let expected = Duration::from_secs(60);
        let slack = Duration::from_secs(2);
        assert!(
            parsed >= expected.saturating_sub(slack) && parsed <= expected + slack,
            "expected ~60s, got {parsed:?}"
        );
    }

    #[test]
    fn http_date_in_past_returns_none() {
        let target = SystemTime::now() - Duration::from_secs(60);
        let formatted = httpdate::fmt_http_date(target);
        let v = header(&formatted);
        assert_eq!(parse_retry_after(Some(&v)), None);
    }

    #[test]
    fn malformed_returns_none() {
        let v = header("not a number and not a date");
        assert_eq!(parse_retry_after(Some(&v)), None);
    }

    #[test]
    fn absent_returns_none() {
        assert_eq!(parse_retry_after(None), None);
    }
}
