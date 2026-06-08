// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! `StateStore` semantic invariance across implementations.
//!
//! Property: for any sequence of `put` / `delete` operations, the
//! final per-key values returned by `get` match a reference
//! `HashMap` model. The same generic body runs against `MemoryStore`
//! and `JsonFileStore` so both impls are proven observationally
//! identical.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test code: unwrap and panic on unexpected variant are the standard test diagnostics"
)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use aviso::state::{Checkpoint, JsonFileStore, MemoryStore, ResumeKey, StateStore};
use proptest::collection::vec;
use proptest::prelude::*;
use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::{Builder, Runtime};
use url::Url;

thread_local! {
    /// Lazy per-thread tokio runtime so proptest cases reuse one
    /// `current_thread` runtime instead of building a fresh one for
    /// every generated case. Without this, `with_cases(64)` across
    /// two test functions builds 128 runtimes per `cargo test` run,
    /// which is wall-clock time spent on setup rather than on
    /// exercising store behaviour.
    static RT: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    RT.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Some(Builder::new_current_thread().enable_all().build().unwrap());
        }
        cell.borrow().as_ref().unwrap().block_on(fut)
    })
}

#[derive(Debug, Clone)]
enum Op {
    Put(u8, u64),
    Delete(u8),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (any::<u8>(), any::<u64>()).prop_map(|(k, v)| Op::Put(k, v)),
        any::<u8>().prop_map(Op::Delete),
    ]
}

fn op_sequence() -> impl Strategy<Value = Vec<Op>> {
    vec(op_strategy(), 0..32)
}

fn key(n: u8) -> ResumeKey {
    ResumeKey::new(
        &Url::parse("https://a/").unwrap(),
        "mars",
        &json!({ "n": n }),
        None,
    )
    .unwrap()
}

async fn apply_and_verify<S: StateStore>(store: &S, ops: Vec<Op>) {
    let mut model: HashMap<u8, u64> = HashMap::new();
    let mut touched: HashSet<u8> = HashSet::new();
    for op in ops {
        match op {
            Op::Put(k, v) => {
                store.put(&key(k), Checkpoint::new(v, None)).await.unwrap();
                // Mirror the strict-monotonic put contract: a put
                // with sequence <= existing is a no-op. Resets
                // require an explicit Delete in the op sequence.
                match model.get(&k) {
                    Some(existing) if *existing >= v => {}
                    _ => {
                        model.insert(k, v);
                    }
                }
                touched.insert(k);
            }
            Op::Delete(k) => {
                store.delete(&key(k)).await.unwrap();
                model.remove(&k);
                touched.insert(k);
            }
        }
    }
    for (k, expected) in &model {
        let got = store.get(&key(*k)).await.unwrap().unwrap();
        assert_eq!(got.last_committed_sequence, *expected);
    }
    // Check absence only for keys actually touched (put-then-deleted).
    // Keys never touched trivially return None for any correct store;
    // iterating the full u8 range every case was wasted work.
    for k in &touched {
        if !model.contains_key(k) {
            assert!(store.get(&key(*k)).await.unwrap().is_none());
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn memory_store_matches_reference_model(ops in op_sequence()) {
        block_on(async {
            let store = MemoryStore::new();
            apply_and_verify(&store, ops).await;
        });
    }

    #[test]
    fn file_store_matches_reference_model(ops in op_sequence()) {
        block_on(async {
            let dir = TempDir::new().unwrap();
            let path: PathBuf = dir.path().join("state.json");
            let store = JsonFileStore::open(&path).await.unwrap();
            apply_and_verify(&store, ops).await;
        });
    }
}
