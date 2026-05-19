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
