// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! The `aviso` command-line binary.
//!
//! A thin shim over [`aviso_cli::run`]. All behaviour lives in the library so
//! the `pyaviso` wheel's bundled `aviso` console command can share the exact
//! same code path; this file owns only the final [`std::process::exit`] that a
//! library must not call itself.

fn main() {
    std::process::exit(aviso_cli::run(std::env::args_os()));
}
