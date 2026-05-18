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

use std::collections::HashMap;
use std::path::PathBuf;

use aviso::state::{Checkpoint, JsonFileStore, MemoryStore, ResumeKey, StateStore};
use proptest::collection::vec;
use proptest::prelude::*;
use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::Builder;
use url::Url;

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
    for op in ops {
        match op {
            Op::Put(k, v) => {
                store.put(&key(k), Checkpoint::new(v, None)).await.unwrap();
                model.insert(k, v);
            }
            Op::Delete(k) => {
                store.delete(&key(k)).await.unwrap();
                model.remove(&k);
            }
        }
    }
    for (k, expected) in &model {
        let got = store.get(&key(*k)).await.unwrap().unwrap();
        assert_eq!(got.last_committed_sequence, *expected);
    }
    for k in 0..=u8::MAX {
        if !model.contains_key(&k) {
            assert!(store.get(&key(k)).await.unwrap().is_none());
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn memory_store_matches_reference_model(ops in op_sequence()) {
        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let store = MemoryStore::new();
            apply_and_verify(&store, ops).await;
        });
    }

    #[test]
    fn file_store_matches_reference_model(ops in op_sequence()) {
        let rt = Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let dir = TempDir::new().unwrap();
            let path: PathBuf = dir.path().join("state.json");
            let store = JsonFileStore::open(&path).await.unwrap();
            apply_and_verify(&store, ops).await;
        });
    }
}
