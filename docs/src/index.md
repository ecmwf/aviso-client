# aviso

aviso is a notification client for [aviso-server](https://github.com/ecmwf/aviso-server), ECMWF's pub-sub service for data-driven workflows. It tells you, in close to real time, when a dataset you care about has just landed on the server.

You can use it three ways. Pick the one that fits how you work.

## From the command line

The `aviso` binary publishes notifications, listens for new ones, and replays history. It runs on Linux, macOS, and Windows.

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

Start with the [CLI overview](./cli/overview.md).

## From a Rust program

The Rust library powers the CLI and is on crates.io. Use it when you want notifications inside a Rust binary or service.

```rust,ignore
use aviso::AvisoClient;

let client = AvisoClient::builder()
    .base_url("https://aviso.example")
    .build()?;
```

Start with the [library guide](./developers/lib-guide.md).

## From Python

A Python package is planned. Until then, the CLI does what you need and works well from Python through `subprocess`.

```python
import subprocess
subprocess.run(["aviso", "listen", "--event", "mars",
                "--identifiers", '{"class":"od"}'])
```

See the [Python page](./python/status.md) for the full pattern.

## New here?

- [What aviso does](./getting-started/overview.md) and how the pieces fit together.
- [Install](./getting-started/install.md) the CLI or the library.
- [Quickstart](./getting-started/quickstart.md): your first publish and your first listener.

## Reference

- [CLI flags](./reference/cli-flags.md): every flag, every subcommand.
- [Listener YAML](./reference/listener-yaml.md): the file format the CLI reads.
- [State file](./reference/state-file.md): what aviso writes to disk to remember where it left off.
- [Rust API](./reference/rust-api.md): types, traits, and functions.
