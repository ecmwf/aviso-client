// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

use super::*;

#[test]
fn gap_guard_no_resume_adopts_first_observed_sequence() {
    let mut g = GapGuard::starting_from(None);
    assert!(g.observe(100).is_ok());
    assert!(g.observe(101).is_ok());
    assert!(matches!(
        g.observe(105),
        Err(GapReason::SequenceJump {
            expected: 102,
            observed: 105
        })
    ));
}

#[test]
fn gap_guard_after_sequence_expects_next() {
    let mut g = GapGuard::starting_from(Some(&ResumeStart::AfterSequence(9)));
    assert!(g.observe(10).is_ok());
    assert!(g.observe(11).is_ok());
    assert!(matches!(
        g.observe(13),
        Err(GapReason::SequenceJump {
            expected: 12,
            observed: 13
        })
    ));
}

#[test]
fn gap_guard_tolerates_backwards_sequence() {
    let mut g = GapGuard::starting_from(None);
    assert!(g.observe(100).is_ok());
    assert!(
        g.observe(50).is_ok(),
        "backwards observation must not be reported as a gap"
    );
}

#[test]
fn gap_guard_overflow_resets_to_fresh_start() {
    let mut g = GapGuard::starting_from(Some(&ResumeStart::AfterSequence(u64::MAX)));
    assert!(
        g.observe(0).is_ok(),
        "overflow on construction must not poison subsequent observation"
    );
}

#[test]
fn gap_guard_relaxed_does_not_report_sequence_jump_as_error() {
    let mut g = GapGuard::relaxed_starting_from(Some(&ResumeStart::AfterSequence(15)));
    assert!(
        g.observe(24).is_ok(),
        "relaxed mode must NOT report a jump from expected=16 to observed=24 as an error; the server filtered events 16-23 server-side and the gap is expected behaviour for filtered listeners"
    );
    assert!(
        g.observe(25).is_ok(),
        "after a relaxed jump, the next consecutive observation continues without complaint"
    );
}

#[test]
fn gap_guard_relaxed_advances_expected_past_the_jump() {
    let mut g = GapGuard::relaxed_starting_from(Some(&ResumeStart::AfterSequence(15)));
    // Skip 16-23, observed jumps to 24.
    let _ = g.observe(24);
    // The next genuine consecutive observation (25) must NOT itself be reported as a jump.
    assert!(
        g.observe(25).is_ok(),
        "relaxed mode must advance `expected` past the jumped-to sequence so the next consecutive observation does not also fire"
    );
    // A SECOND jump after the relaxed advance is also tolerated.
    assert!(g.observe(40).is_ok());
}

#[test]
fn gap_guard_strict_default_still_reports_jumps_for_unfiltered_listeners() {
    // Regression guard: the `starting_from` constructor must STILL
    // be strict (the existing public contract for unfiltered
    // listeners where every event must be delivered).
    let mut g = GapGuard::starting_from(Some(&ResumeStart::AfterSequence(15)));
    assert!(matches!(
        g.observe(24),
        Err(GapReason::SequenceJump {
            expected: 16,
            observed: 24
        })
    ));
}
