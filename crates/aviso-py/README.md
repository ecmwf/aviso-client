# aviso-py

PyO3 binding crate for [`aviso`](https://crates.io/crates/aviso), the core Rust client library for [`aviso-server`](https://github.com/ecmwf/aviso-server).

Python users should install the [`aviso` distribution from PyPI](https://pypi.org/project/aviso/), not this crate directly. This crate exists so the Python wheel can be built from the Rust workspace.

> Status: bindings scaffold. The crate currently builds as a Rust `rlib` placeholder; it will become a PyO3 `cdylib` once bindings land.

See the [workspace repository](https://github.com/ecmwf/aviso-client) for the full client suite and architectural background.

## License

Apache-2.0. See [`LICENSE.txt`](https://github.com/ecmwf/aviso-client/blob/main/LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
