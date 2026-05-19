use std::collections::HashMap;

use crate::state::{Checkpoint, ResumeKey};

/// Merge `candidate` against the freshly re-read `disk_state` so no
/// committed checkpoint moves backwards, ever.
///
/// `last_committed_sequence` is the load-bearing forward-only cursor
/// the supervisor reconnects from. The merge enforces three rules:
///
/// 1. Key in candidate AND disk's sequence is greater than OR equal
///    to candidate's: disk wins. ("No checkpoint goes backwards"
///    applies uniformly, whether the higher value came from another
///    process or from this handle's own earlier put. The
///    equal-sequence case also yields to disk so the visible value
///    matches the trait's "put with sequence <= existing is a
///    silent no-op" contract for both `MemoryStore` and
///    `JsonFileStore`.)
/// 2. Key in disk but NOT in candidate AND NOT in `deletes`:
///    preserve disk (another process's write this handle has never
///    observed, or this handle's own earlier put that has not yet
///    propagated into this candidate).
/// 3. Key in `deletes`: if disk has it AND disk's sequence is
///    greater than the pre-state sequence we recorded at delete
///    time, the delete is suppressed and disk's value is preserved
///    (a concurrent writer advanced the key past what the caller
///    saw, so the "delete the K I knew" intent is no longer well
///    defined). Otherwise the delete is honoured.
///
/// Rule 3 prevents the bug where `DELETE K` against a concurrent
/// `PUT K=N+M` would silently drop the concurrent write. Rule 1
/// applies even when disk matches pre-state because a fresh process
/// that opens an existing store sees pre-state == disk: a `put` with
/// a lower sequence in that situation is still a regression that
/// would silently lose the durable value. Callers that need to
/// reset a checkpoint to a lower sequence must `delete` first, then
/// `put`; a bare `put(k, lower_seq)` silently no-ops by design.
pub(super) fn merge_monotonic(
    mut candidate: HashMap<ResumeKey, Checkpoint>,
    disk_state: HashMap<ResumeKey, Checkpoint>,
    deletes: &HashMap<ResumeKey, u64>,
) -> HashMap<ResumeKey, Checkpoint> {
    for (k, disk_cp) in disk_state {
        if let Some(&pre_seq) = deletes.get(&k) {
            if disk_cp.last_committed_sequence > pre_seq {
                candidate.insert(k, disk_cp);
            }
            continue;
        }
        match candidate.get(&k) {
            Some(cand) if disk_cp.last_committed_sequence >= cand.last_committed_sequence => {
                candidate.insert(k, disk_cp);
            }
            Some(_) => {}
            None => {
                candidate.insert(k, disk_cp);
            }
        }
    }
    candidate
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]
mod tests {
    use serde_json::json;
    use url::Url;

    use std::collections::HashMap;

    use super::merge_monotonic;
    use crate::state::{Checkpoint, ResumeKey};

    fn key(n: u8) -> ResumeKey {
        ResumeKey::new(
            &Url::parse("https://a/").unwrap(),
            "mars",
            &json!({"n": n}),
            None,
        )
        .unwrap()
    }

    fn cp(seq: u64) -> Checkpoint {
        Checkpoint::new(seq, None)
    }

    fn map(pairs: &[(u8, u64)]) -> HashMap<ResumeKey, Checkpoint> {
        pairs.iter().map(|(k, s)| (key(*k), cp(*s))).collect()
    }

    fn deletes(pairs: &[(u8, u64)]) -> HashMap<ResumeKey, u64> {
        pairs.iter().map(|(k, s)| (key(*k), *s)).collect()
    }

    #[test]
    fn merge_empty_disk_and_empty_candidate_is_empty() {
        let result = merge_monotonic(map(&[]), map(&[]), &deletes(&[]));
        assert!(result.is_empty());
    }

    #[test]
    fn merge_empty_disk_keeps_candidate_as_is() {
        let result = merge_monotonic(map(&[(1, 5)]), map(&[]), &deletes(&[]));
        assert_eq!(result.get(&key(1)).unwrap().last_committed_sequence, 5);
    }

    #[test]
    fn merge_preserves_disk_keys_unknown_to_candidate() {
        let result = merge_monotonic(map(&[]), map(&[(1, 7)]), &deletes(&[]));
        assert_eq!(result.get(&key(1)).unwrap().last_committed_sequence, 7);
    }

    #[test]
    fn merge_disk_wins_when_disk_seq_greater_than_candidate_seq() {
        // Strict monotonic: a lower-sequence put never overwrites a
        // higher-sequence on-disk value, even when the on-disk value
        // matches this handle's pre-state. Callers that need to
        // reset must delete first.
        let result = merge_monotonic(map(&[(1, 0)]), map(&[(1, 9)]), &deletes(&[]));
        assert_eq!(result.get(&key(1)).unwrap().last_committed_sequence, 9);
    }

    #[test]
    fn merge_candidate_wins_when_candidate_seq_greater_than_disk_seq() {
        let result = merge_monotonic(map(&[(1, 15)]), map(&[(1, 10)]), &deletes(&[]));
        assert_eq!(result.get(&key(1)).unwrap().last_committed_sequence, 15);
    }

    #[test]
    fn merge_disk_wins_when_seqs_are_equal() {
        // The trait contract is "put with sequence <= existing is a
        // silent no-op". On equal sequences, disk wins; the visible
        // value (last_committed_sequence) is identical so callers
        // observe a no-op even when the candidate's metadata (e.g.,
        // a slightly different event_id from a parallel writer)
        // differs.
        let result = merge_monotonic(map(&[(1, 5)]), map(&[(1, 5)]), &deletes(&[]));
        assert_eq!(result.get(&key(1)).unwrap().last_committed_sequence, 5);
    }

    #[test]
    fn merge_honours_delete_when_disk_seq_matches_pre_state() {
        // pre={K=5}, mutator deletes K, disk K=5 (matches pre).
        // No concurrent writer; honour the delete.
        let result = merge_monotonic(map(&[]), map(&[(1, 5)]), &deletes(&[(1, 5)]));
        assert!(result.is_empty());
    }

    #[test]
    fn merge_suppresses_delete_when_disk_advanced_past_pre_state() {
        // pre={K=5}, mutator deletes K, disk K=10 (concurrent
        // writer). 10 > 5 so delete is suppressed; disk wins.
        let result = merge_monotonic(map(&[]), map(&[(1, 10)]), &deletes(&[(1, 5)]));
        assert_eq!(result.get(&key(1)).unwrap().last_committed_sequence, 10);
    }

    #[test]
    fn merge_honours_delete_when_disk_has_no_key() {
        // pre={K=5}, mutator deletes K, disk has no K (concurrent
        // writer also deleted, or never existed). Honour delete.
        let result = merge_monotonic(map(&[]), map(&[]), &deletes(&[(1, 5)]));
        assert!(result.is_empty());
    }

    #[test]
    fn merge_mixed_put_and_delete_against_concurrent_writes() {
        // pre={K=2, L=3}, mutator put L=20 + delete K.
        // disk has K=10, L=5 (concurrent writes on both).
        // Expected: K=10 preserved (delete suppressed), L=20 kept
        // (candidate is monotonically ahead of disk's L=5).
        let result = merge_monotonic(
            map(&[(2, 20)]),
            map(&[(1, 10), (2, 5)]),
            &deletes(&[(1, 2)]),
        );
        assert_eq!(result.get(&key(1)).unwrap().last_committed_sequence, 10);
        assert_eq!(result.get(&key(2)).unwrap().last_committed_sequence, 20);
    }
}
