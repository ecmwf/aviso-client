# Developers

This section is for people working on aviso itself, or embedding the `aviso`
Rust library in their own program. If you just want to use the CLI, start at
[CLI overview](../cli/overview.md) instead.

## What lives here

- [Architecture](./architecture.md): how the crates fit together and where each
  responsibility lives.
- [Library guide](./lib-guide.md): how to use the `aviso` Rust crate from your
  own code.
- [Contributing](./contributing.md): how to make a change to this repository:
  tests, gates, the local workflow.
- [Docs style guide](./docs-style.md): the voice and the conventions
  documentation pages follow.

The CLI and the future Python package are both consumers of the `aviso` core
library. The core library does not depend on either of them. The
[Architecture](./architecture.md) page draws the relationship.
