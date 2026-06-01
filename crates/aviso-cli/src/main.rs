//! The `aviso` command-line binary.
//!
//! A thin shim over [`aviso_cli::run`]. All behaviour lives in the library so
//! the `pyaviso` wheel's bundled `aviso` console command can share the exact
//! same code path; this file owns only the final [`std::process::exit`] that a
//! library must not call itself.

fn main() {
    std::process::exit(aviso_cli::run(std::env::args_os()));
}
