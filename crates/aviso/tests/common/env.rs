// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The environment harness shared by the tests that read the config file.

#![allow(
    clippy::expect_used,
    reason = "test code: expect on fixture setup is the expected diagnostic"
)]

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Points every source at `dir` and restores the previous values on drop.
pub struct Sources {
    _guard: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl Sources {
    pub fn in_dir(dir: &Path) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let names = [
            "AVISO_TOKEN",
            "AVISO_USERNAME",
            "AVISO_PASSWORD",
            "AVISO_BASE_URL",
            "AVISO_CLIENT_CONFIG_FILE",
            "AVISO_CREDENTIALS_FILE",
        ];
        let saved = names.iter().map(|k| (*k, std::env::var_os(k))).collect();
        // SAFETY: ENV_LOCK is held, so no other test in this binary reads or
        // writes these variables while they are changed.
        unsafe {
            for name in &names[..4] {
                std::env::remove_var(name);
            }
            std::env::set_var("AVISO_CLIENT_CONFIG_FILE", dir.join("config.yaml"));
            std::env::set_var(
                "AVISO_CREDENTIALS_FILE",
                dir.join("absent-credentials.yaml"),
            );
        }
        Self {
            _guard: guard,
            saved,
        }
    }
}

impl Drop for Sources {
    fn drop(&mut self) {
        // SAFETY: the lock is still held for the lifetime of this value.
        unsafe {
            for (name, value) in &self.saved {
                match value {
                    Some(v) => std::env::set_var(name, v),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

pub fn write_config(dir: &Path, body: &str) {
    std::fs::write(dir.join("config.yaml"), body).expect("write config");
}
