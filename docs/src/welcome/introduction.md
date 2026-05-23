# Introduction

`aviso-client` is the client suite for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows. The repository is `aviso-client`; the crates inside are short and unprefixed because they live in their own namespace:

- a **Rust library crate** (`aviso`) that is the canonical implementation;
- a **Rust CLI binary** (`aviso`, produced by the `aviso-cli` crate) for scripted and interactive use;
- a **Python package** (`aviso`, with PyO3 bindings in the `aviso-py` crate) that is the primary user-facing surface for most consumers.

## Where to start

- **Understanding what aviso does**: read [Key concepts](./key-concepts.md).
- **Tracking the plan**: see [`plans/`](https://github.com/ecmwf/aviso-client/tree/main/plans) at the repo root.
- **Contributing**: read [Contributing](../internals/contributing.md) and [`AGENTS.md`](https://github.com/ecmwf/aviso-client/blob/main/AGENTS.md) at the repo root.
