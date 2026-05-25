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

The `aviso` package wraps the Rust core through PyO3, so you get a real Python API: typed value objects, structured exceptions, `for`-iteration over a watch stream, and the same trigger surface the CLI uses.

```python
import os
import aviso

client = aviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=aviso.Env())

for notification in client.listen("mars", filter={"class": "od"}):
    print(notification.sequence, notification.payload)
```

Start with the [Python overview](./python/overview.md).

## New here?

- [What aviso does](./getting-started/overview.md) and how the pieces fit together.
- [Install](./getting-started/install.md) the CLI or the library.
- [Quickstart](./getting-started/quickstart.md): your first publish and your first listener.

## Reference

- [CLI flags](./reference/cli-flags.md): every flag, every subcommand.
- [Listener YAML](./reference/listener-yaml.md): the file format the CLI reads.
- [State file](./reference/state-file.md): what aviso writes to disk to remember where it left off.
- [Rust API](./reference/rust-api.md): types, traits, and functions.
