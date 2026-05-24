//! Multi-process kill-during-write fuzz harness for `JsonFileStore`.
//!
//! Validates the cross-process lock + monotonic-cursor merge under
//! `SIGKILL` stress. The single compiled test binary acts as both
//! parent (the cargo-test invocation) and child (self-spawned via
//! `std::env::current_exe()` plus the magic `--fuzz-writer-mode`
//! argv and the `AVISO_FUZZ_CHILD` env var). Self-spawning avoids
//! shipping a separate `[[bin]]` helper that would land in
//! `cargo install` output, and the env-var gate prevents accidental
//! child-mode entry from a stray `cargo test ... -- --fuzz-writer-mode`.
//!
//! Test contract (see plan v0.3 §5 Resolved Q8):
//!
//! 1. Spawn N=4 children that loop puts with monotonically
//!    incrementing sequences on their own per-process keyspace.
//! 2. After 500 ms, `SIGKILL` (or `TerminateProcess` on Windows)
//!    every child via `std::process::Child::kill()`.
//! 3. Re-open the store and parse each child's append-only journal.
//!    Assert: stored sequence per child >= max(journaled), file
//!    parses cleanly.
//! 4. Run a second wave starting from sequence 0; assert no key's
//!    stored sequence has decreased across waves (proves the merge
//!    prevented backward motion when the lower-starting writers
//!    raced with the surviving disk state).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    reason = "test code: unwrap, expect, and panic on unexpected variant are the standard test diagnostics"
)]

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};

use aviso::state::{Checkpoint, JsonFileStore, ResumeKey, StateStore};
use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::Builder;
use url::Url;

const FUZZ_FLAG: &str = "--fuzz-writer-mode";
const FUZZ_ENV: &str = "AVISO_FUZZ_CHILD";
const FUZZ_ENV_VALUE: &str = "1";
const NUM_CHILDREN: u8 = 4;
/// How long the parent watches journals for the first child write to
/// appear before declaring an infrastructure failure. Generous because
/// CI runners under contention can take several seconds for the first
/// child to acquire the file lock and complete a put-cycle (read,
/// flock, re-read, merge, atomic rename, release).
const FIRST_WRITE_TIMEOUT: Duration = Duration::from_secs(20);
/// Observation window held open once at least one child has written.
/// During this window every child gets a fair shot at writing more
/// entries, exercising the multi-process lock contention this test
/// targets, before the SIGKILL wave fires.
const OBSERVATION_WINDOW: Duration = Duration::from_millis(200);
/// Poll cadence for the journal-progress watcher. 25 ms is fast
/// enough that the per-poll cost is invisible on a CI runner yet
/// slow enough that the watcher itself does not starve the children.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

fn main() {
    let argv: Vec<OsString> = std::env::args_os().collect();
    let has_fuzz_flag = argv.iter().any(|a| a == FUZZ_FLAG);
    let env_set = std::env::var_os(FUZZ_ENV).is_some();

    if has_fuzz_flag && env_set {
        run_fuzz_writer(&argv);
    } else if has_fuzz_flag {
        // Argv present but env var not set: someone passed the magic
        // flag to cargo test by hand. Refuse loudly rather than
        // misfire into child mode.
        panic!(
            "{FUZZ_FLAG} is reserved for self-spawned fuzz children; \
             do not pass it via `cargo test ... -- {FUZZ_FLAG}`",
        );
    } else {
        run_kill_fuzz_test();
    }
}

fn run_fuzz_writer(argv: &[OsString]) {
    // Args: [exe, --fuzz-writer-mode, <state_path>, <journal_path>,
    //        <child_idx>, <starting_seq>]
    let mut iter = argv.iter().skip_while(|a| a.as_os_str() != FUZZ_FLAG);
    iter.next().expect("FUZZ_FLAG must be present here");
    let state_path = PathBuf::from(iter.next().expect("state path argv"));
    let journal_path = PathBuf::from(iter.next().expect("journal path argv"));
    let child_idx: u8 = iter
        .next()
        .expect("child idx argv")
        .to_string_lossy()
        .parse()
        .expect("child idx parses as u8");
    let starting_seq: u64 = iter
        .next()
        .expect("starting seq argv")
        .to_string_lossy()
        .parse()
        .expect("starting seq parses as u64");

    let rt = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builds");
    rt.block_on(async move {
        let store = JsonFileStore::open(&state_path).await.expect("store opens");
        let mut journal = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&journal_path)
            .expect("journal opens");
        let key = key_for_child(child_idx);
        let mut seq = starting_seq;
        loop {
            store
                .put(&key, Checkpoint::new(seq, None))
                .await
                .expect("put succeeds");
            writeln!(journal, "{seq}").expect("journal write");
            journal.flush().expect("journal flush");
            seq = seq.checked_add(1).expect("seq does not overflow u64");
        }
    });
}

fn run_kill_fuzz_test() {
    let dir = TempDir::new().expect("tempdir creates");
    let state_path = dir.path().join("state.json");

    // ---- First wave -----------------------------------------------------
    let first_journals: Vec<PathBuf> = (0..NUM_CHILDREN)
        .map(|i| dir.path().join(format!("child_{i}_wave1.journal")))
        .collect();
    let mut children = spawn_wave(&state_path, &first_journals, 0);
    wait_for_progress_then_observe(&mut children, &first_journals, "first wave");
    kill_all(&mut children);

    let after_first = read_store(&state_path);
    let mut first_wave_progress = 0usize;
    for (i, journal_path) in first_journals.iter().enumerate() {
        if let Some(max_journaled) = max_in_journal(journal_path) {
            first_wave_progress += 1;
            let stored = after_first
                .get(&key_for_child(u8_try_from(i)))
                .copied()
                .unwrap_or_else(|| {
                    panic!("first-wave child {i} journaled {max_journaled} but store has no entry")
                });
            assert!(
                stored >= max_journaled,
                "first-wave child {i}: stored {stored} < max journaled {max_journaled}",
            );
        }
        // Empty journal means the child was killed before its first
        // successful put. Allowed for individual children; the
        // wave-level progress check below is satisfied by
        // wait_for_progress_then_observe having returned (which
        // requires at least one child to have flushed at least one
        // entry before the observation window starts).
    }
    assert!(
        first_wave_progress > 0,
        "first wave: no child made any progress within {FIRST_WRITE_TIMEOUT:?}; the test infra polled journals before SIGKILL and observed none with content. Either CI is exceptionally slow or the multi-process lock is genuinely starving every child.",
    );

    // ---- Second wave: children start at seq 0 ----------------------------
    let second_journals: Vec<PathBuf> = (0..NUM_CHILDREN)
        .map(|i| dir.path().join(format!("child_{i}_wave2.journal")))
        .collect();
    let mut children = spawn_wave(&state_path, &second_journals, 0);
    wait_for_progress_then_observe(&mut children, &second_journals, "second wave");
    kill_all(&mut children);

    let after_second = read_store(&state_path);
    let mut second_wave_progress = 0usize;
    for i in 0..NUM_CHILDREN {
        let key = key_for_child(i);
        let first_seq = after_first.get(&key).copied();
        let second_seq = after_second.get(&key).copied();
        if let (Some(f), Some(s)) = (first_seq, second_seq) {
            assert!(
                s >= f,
                "child {i}: second-wave stored {s} < first-wave stored {f}; \
                 monotonic-merge across waves failed",
            );
        }
        if let Some(max_journaled) = max_in_journal(&second_journals[usize::from(i)]) {
            second_wave_progress += 1;
            let stored = second_seq.unwrap_or_else(|| {
                panic!("second-wave child {i} journaled {max_journaled} but store has no entry")
            });
            assert!(
                stored >= max_journaled,
                "second-wave child {i}: stored {stored} < max journaled {max_journaled}",
            );
        }
    }
    assert!(
        second_wave_progress > 0,
        "second wave: no child made any progress within {FIRST_WRITE_TIMEOUT:?}; see the first-wave message above for the diagnostic shape.",
    );
}

/// Adaptive wait that replaces a fixed-wall-clock sleep with a poll
/// loop on the children's journal files.
///
/// The function blocks until ONE of the journals has been observed to
/// contain at least one line (proves at least one child completed a
/// put-cycle including its journal flush), then holds open an
/// observation window so siblings get a fair chance to write more
/// entries before the SIGKILL wave fires. Caps the first-write wait
/// at [`FIRST_WRITE_TIMEOUT`]; if no child has written by then the
/// caller's assertion surfaces a genuine infrastructure failure.
///
/// Why adaptive: a fixed 500 ms wall-clock window was vacuous on
/// slow CI runners where the four children all serialise through the
/// single `fd-lock` on `state.json.lock`. With four children
/// competing for one flock, the per-put-cycle wall time (open the
/// data file, acquire flock, re-read disk state, merge the in-memory
/// state, write the temp file, fsync, rename, release flock) under
/// CI contention can exceed 500 ms for every child, so the kill wave
/// fires before anyone has flushed. Polling the journals makes the
/// test robust to slow runners while keeping the contention-real
/// invariant the original window aimed at.
fn wait_for_progress_then_observe(children: &mut [Child], journals: &[PathBuf], wave_label: &str) {
    let start = Instant::now();
    let mut first_progress_at: Option<Instant> = None;

    loop {
        thread::sleep(POLL_INTERVAL);

        // Fail loudly if any child died early (panic, OOM, race in
        // the dispatch loop). Without this guard a dead child leaves
        // an empty journal and the per-child progress assertions
        // silently skip it.
        assert_all_still_running(children, wave_label);

        if first_progress_at.is_none() && any_journal_nonempty(journals) {
            first_progress_at = Some(Instant::now());
        }

        match first_progress_at {
            Some(t) if t.elapsed() >= OBSERVATION_WINDOW => return,
            None if start.elapsed() >= FIRST_WRITE_TIMEOUT => return,
            _ => {}
        }
    }
}

fn any_journal_nonempty(journals: &[PathBuf]) -> bool {
    journals
        .iter()
        .any(|p| std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
}

fn assert_all_still_running(children: &mut [Child], wave_label: &str) {
    // Vacuous-pass guard: if any child has already exited before the
    // SIGKILL window expires, the test setup is broken (the child
    // panicked, OOM'd, or hit a race that ended the loop). Without
    // this check, a child that exits immediately leaves an empty
    // journal and the per-child progress assertions silently skip
    // it, which can mask real bugs in the dispatch loop.
    for (i, child) in children.iter_mut().enumerate() {
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) => panic!(
                "{wave_label} child {i} exited before SIGKILL with status {status:?}; \
                 expected the child to loop until killed",
            ),
            Err(e) => panic!("{wave_label} child {i} try_wait error: {e}"),
        }
    }
}

fn spawn_wave(state_path: &Path, journals: &[PathBuf], starting_seq: u64) -> Vec<Child> {
    let exe = std::env::current_exe().expect("current exe");
    (0..NUM_CHILDREN)
        .map(|i| {
            Command::new(&exe)
                .arg(FUZZ_FLAG)
                .arg(state_path)
                .arg(&journals[usize::from(i)])
                .arg(i.to_string())
                .arg(starting_seq.to_string())
                .env(FUZZ_ENV, FUZZ_ENV_VALUE)
                .spawn()
                .unwrap_or_else(|e| panic!("spawn child {i}: {e}"))
        })
        .collect()
}

fn kill_all(children: &mut [Child]) {
    for (i, child) in children.iter_mut().enumerate() {
        // Child::kill on Unix sends SIGKILL; on Windows it calls
        // TerminateProcess. Both release any flock the child held
        // immediately via kernel cleanup. EBADF means the child is
        // already gone; ignore that.
        match child.kill() {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
            Err(e) => panic!("kill child {i}: {e}"),
        }
        child
            .wait()
            .unwrap_or_else(|e| panic!("wait child {i}: {e}"));
    }
}

fn read_store(state_path: &Path) -> std::collections::HashMap<ResumeKey, u64> {
    let rt = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builds");
    rt.block_on(async move {
        let store = JsonFileStore::open(state_path)
            .await
            .expect("post-kill store opens cleanly");
        let mut out = std::collections::HashMap::new();
        for i in 0..NUM_CHILDREN {
            let k = key_for_child(i);
            if let Some(cp) = store.get(&k).await.expect("get succeeds") {
                out.insert(k, cp.last_committed_sequence);
            }
        }
        out
    })
}

fn max_in_journal(path: &Path) -> Option<u64> {
    let file = std::fs::File::open(path).ok()?;
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| line.trim().parse::<u64>().ok())
        .max()
}

fn key_for_child(idx: u8) -> ResumeKey {
    ResumeKey::new(
        &Url::parse("https://kill-fuzz/").expect("url parses"),
        "mars",
        &json!({ "child": idx }),
        None,
    )
    .expect("resume key derives")
}

fn u8_try_from(n: usize) -> u8 {
    u8::try_from(n).expect("child index fits in u8")
}
