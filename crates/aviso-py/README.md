# aviso-py

PyO3 binding crate for [`aviso`](https://crates.io/crates/aviso), the core Rust client library for [`aviso-server`](https://github.com/ecmwf/aviso-server).

Python users should install the [`aviso` distribution](https://github.com/ecmwf/aviso-client/tree/main/python) (built from this crate via [`maturin`](https://maturin.rs/)), not depend on this crate directly. This crate exists so the Python wheel can be built from the Rust workspace; it builds as a `cdylib` (the Python extension `aviso._native`) plus an `rlib` (so the workspace `cargo build`/`cargo test` can see the crate's types in cross-crate tests).

The crate exposes the full Python surface: synchronous and asynchronous clients, value types, the trigger builder, auth providers, state stores, listening iterators, and the exception hierarchy. See the [workspace repository](https://github.com/ecmwf/aviso-client) for the full client suite and the [Python documentation](https://github.com/ecmwf/aviso-client/tree/main/docs/src/python) for the user-facing API.

## License

Apache-2.0. See [`LICENSE.txt`](https://github.com/ecmwf/aviso-client/blob/main/LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
