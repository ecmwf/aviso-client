# Developers

This section is for people working on aviso itself, or embedding the `aviso` Rust library in their own program. It is not the right place to start if you just want to use the CLI; head to [CLI overview](../cli/overview.md) for that.

## What lives here

- [Architecture](./architecture.md): how the crates fit together and where each responsibility lives.
- [Library guide](./lib-guide.md): how to use the `aviso` Rust crate from your own code.
- [Contributing](./contributing.md): how to make a change to this repository: tests, gates, the local workflow.
- [Docs style guide](./docs-style.md): the voice and the conventions documentation pages follow.

## The repository map

```text
aviso-client/
├── crates/
│   ├── aviso/          The core library. Published as `aviso`.
│   ├── aviso-cli/      The command-line binary. Published as `aviso-cli`.
│   ├── aviso-py/       Future Python extension crate (placeholder rlib today).
│   └── finesse/        Internal SSE parser. Not published.
├── python/
│   └── aviso/          Pure-Python helpers that will ship alongside the extension.
├── docs/               This book.
├── plans/              Planning documents.
└── tests/              Workspace integration tests.
```

The CLI and the future Python package are both consumers of the `aviso` core library. The core library does not depend on either of them.
