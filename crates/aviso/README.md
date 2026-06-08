# aviso

Core Rust client library for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows.

This crate carries the foundation: low-level types, transport, authentication, reconnect, and triggers. Companion crates wrap it for specific consumers:

- [`aviso-cli`](https://crates.io/crates/aviso-cli): command-line tool.
- [`pyaviso`](https://pypi.org/project/pyaviso/): Python distribution, built from the [`aviso-py`](https://github.com/ecmwf/aviso-client/tree/main/crates/aviso-py) PyO3 binding crate (published to PyPI, not crates.io).

See the [workspace repository](https://github.com/ecmwf/aviso-client) and the [docs](https://github.com/ecmwf/aviso-client/tree/main/docs) for full documentation. Architectural decisions live in [`plans/decisions.md`](https://github.com/ecmwf/aviso-client/blob/main/plans/decisions.md).

## License

Apache-2.0. See [`LICENSE.txt`](https://github.com/ecmwf/aviso-client/blob/main/LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
