// (C) Copyright 2024- ECMWF and individual contributors.
//
// This software is licensed under the terms of the Apache Licence Version 2.0
// which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
// In applying this licence, ECMWF does not waive the privileges and immunities
// granted to it by virtue of its status as an intergovernmental organisation nor
// does it submit to any jurisdiction.

//! Subcommand handlers for the `aviso` binary.
//!
//! Each submodule owns one of the nine subcommands. This module
//! file is the seam that `main.rs` dispatches through.

pub(crate) mod admin;
pub(crate) mod completions;
pub(crate) mod config_dump;
pub(crate) mod listen;
pub(crate) mod notify;
pub(crate) mod replay;
pub(crate) mod schema;
