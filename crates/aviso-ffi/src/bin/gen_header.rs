// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Regenerates the checked-in C header (`include/aviso.h`) from the crate's
//! `extern "C"` surface. Built and run only with the `gen-header` feature:
//!
//! ```text
//! cargo run -p aviso-ffi --features gen-header --bin gen-header
//! ```
//!
//! CI regenerates into a temporary path and diffs it against the committed
//! header, so a surface change that is not regenerated fails the build.

use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let include_dir = crate_dir.join("include");
    std::fs::create_dir_all(&include_dir)?;
    let output = include_dir.join("aviso.h");

    let config = cbindgen::Config::from_root_or_default(&crate_dir);
    let bindings = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()?;
    bindings.write_to_file(&output);
    Ok(())
}
