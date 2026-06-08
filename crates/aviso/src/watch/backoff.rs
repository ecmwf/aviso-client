// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Reconnect backoff calculator.
//!
//! Pure function mapping `(attempt, ReconnectPolicy)` to a `Duration`.
//! Owns no I/O, no async, no state outside a single startup `Instant`
//! and an `AtomicU64` that stir the jitter source across calls. The
//! watch supervisor consumes this from inside its outer reconnect loop.
//!
//! D2 specifies the policy semantics:
//! - `Immediate` returns zero (routine `max_duration_reached` closes,
//!   `end_of_stream` in watch mode).
//! - `ShortBackoff` returns a fixed five seconds (single-digit per D2;
//!   used for `server_shutdown`).
//! - `ExponentialBackoff` returns a uniformly distributed value in
//!   `[0, min(BASE_DELAY_MS * 2^attempt, MAX_DELAY))` (half-open;
//!   modulo arithmetic gives an exclusive upper bound), AWS-style
//!   full jitter (transport errors and heartbeat starvation).
//!
//! The jitter mixer is intentionally not cryptographic. Full-jitter
//! does not require uniform randomness; the requirement is that a
//! fleet of clients reconnecting after a shared outage spreads their
//! retries across a window so the server is not stampeded. Any
//! "different enough" source suffices. The mixer combines a
//! startup-`Instant` elapsed-nanoseconds reading with a monotonic
//! atomic counter, then applies one round of splitmix64 to break up
//! the bit pattern.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::ReconnectPolicy;

/// Base delay in milliseconds for the exponential schedule.
const BASE_DELAY_MS: u64 = 250;

/// Hard cap on any computed exponential backoff (D2).
const MAX_DELAY: Duration = Duration::from_secs(30);

/// Fixed delay for `ShortBackoff` (D2: single-digit seconds for
/// `server_shutdown`).
const SHORT_BACKOFF: Duration = Duration::from_secs(5);

/// Compute the next reconnect delay.
///
/// `attempt` is the number of consecutive failed connection attempts.
/// `attempt = 0` is the first attempt after a healthy session ended;
/// `attempt = N` for N greater than zero applies the doubled window
/// up to the `MAX_DELAY` cap.
#[must_use]
pub(crate) fn compute_backoff(attempt: u32, policy: ReconnectPolicy) -> Duration {
    match policy {
        ReconnectPolicy::Immediate => Duration::ZERO,
        ReconnectPolicy::ShortBackoff => SHORT_BACKOFF,
        ReconnectPolicy::ExponentialBackoff => {
            // Clamp the shift to avoid undefined behaviour on extreme inputs.
            // 2^20 already exceeds the MAX_DELAY in milliseconds by orders of
            // magnitude; anything beyond saturates at the cap regardless.
            let bounded_attempt = attempt.min(20);
            let ceil_ms = BASE_DELAY_MS.saturating_mul(1u64 << bounded_attempt);
            let ceil = Duration::from_millis(ceil_ms).min(MAX_DELAY);
            let ceil_nanos = u64::try_from(ceil.as_nanos()).unwrap_or(u64::MAX);
            if ceil_nanos == 0 {
                Duration::ZERO
            } else {
                Duration::from_nanos(next_jitter_u64() % ceil_nanos)
            }
        }
    }
}

/// Stir a startup-`Instant` elapsed reading with a monotonic counter
/// through one splitmix64 round.
///
/// Two clients started in the same nanosecond will diverge on the
/// second call because the counter increments. Within a single
/// client, successive calls produce different values for the same
/// reason. The output is not cryptographically uniform but is more
/// than adequate for spreading reconnect attempts across a window.
fn next_jitter_u64() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let start = START.get_or_init(Instant::now);
    let nanos = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut x = nanos.wrapping_add(count.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::Duration;

    use super::{BASE_DELAY_MS, MAX_DELAY, ReconnectPolicy, SHORT_BACKOFF, compute_backoff};

    #[test]
    fn immediate_returns_zero() {
        assert_eq!(
            compute_backoff(0, ReconnectPolicy::Immediate),
            Duration::ZERO
        );
        assert_eq!(
            compute_backoff(99, ReconnectPolicy::Immediate),
            Duration::ZERO
        );
    }

    #[test]
    fn short_backoff_returns_fixed_5s() {
        assert_eq!(
            compute_backoff(0, ReconnectPolicy::ShortBackoff),
            SHORT_BACKOFF
        );
        assert_eq!(
            compute_backoff(7, ReconnectPolicy::ShortBackoff),
            SHORT_BACKOFF
        );
    }

    #[test]
    fn exponential_zero_attempt_within_base() {
        for _ in 0..100 {
            let d = compute_backoff(0, ReconnectPolicy::ExponentialBackoff);
            assert!(d <= Duration::from_millis(BASE_DELAY_MS), "got {d:?}");
        }
    }

    #[test]
    fn exponential_attempt_one_within_500ms() {
        for _ in 0..100 {
            let d = compute_backoff(1, ReconnectPolicy::ExponentialBackoff);
            assert!(d <= Duration::from_millis(BASE_DELAY_MS * 2), "got {d:?}");
        }
    }

    #[test]
    fn exponential_caps_at_max_delay() {
        for _ in 0..100 {
            let d = compute_backoff(100, ReconnectPolicy::ExponentialBackoff);
            assert!(d <= MAX_DELAY, "got {d:?}");
        }
    }

    #[test]
    fn exponential_handles_u32_overflow_in_attempt() {
        for _ in 0..100 {
            let d = compute_backoff(u32::MAX, ReconnectPolicy::ExponentialBackoff);
            assert!(
                d <= MAX_DELAY,
                "must not panic and must respect cap; got {d:?}"
            );
        }
    }

    #[test]
    fn exponential_two_calls_differ_in_expectation() {
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            seen.insert(compute_backoff(10, ReconnectPolicy::ExponentialBackoff));
        }
        assert!(
            seen.len() >= 100,
            "1000 calls must produce at least 100 distinct durations; \
             got {} distinct values",
            seen.len()
        );
    }
}
