//! Subcommand handlers for the `aviso` binary.
//!
//! Each submodule owns one of the nine subcommands. This module
//! file is the seam that `main.rs` dispatches through.

pub(crate) mod completions;
pub(crate) mod config_dump;
pub(crate) mod listen;
pub(crate) mod notify;
pub(crate) mod replay;
pub(crate) mod schema;
