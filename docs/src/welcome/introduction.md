# Introduction

`aviso-client` is the client suite for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows. It ships in three forms:

- a **Rust library crate** (`aviso-client`) that is the canonical implementation;
- a **Rust CLI binary** (`aviso-client-cli`) for scripted and interactive use;
- a **Python package** (`aviso-client`) that wraps the Rust core via PyO3 and is the primary user-facing surface for most consumers.

## Status

Phase 0 — repository bootstrap. The workspace, build system, and documentation skeleton are in place; no user-facing functionality has been implemented yet. See [Architectural decisions](../internals/decisions.md) for the phased roadmap.

## Where to start

- **Trying it out**: skip ahead to [Quick start](./quick-start.md). It will be empty until Phase 1.
- **Understanding what aviso does**: read [Key concepts](./key-concepts.md).
- **Contributing**: read [Contributing](../internals/contributing.md) and [`AGENTS.md`](https://github.com/ecmwf/aviso-client/blob/main/AGENTS.md) at the repo root.
