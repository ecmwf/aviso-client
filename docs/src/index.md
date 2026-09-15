<div align="center">
  <img class="logo-light" src="./images/logo_light.svg" alt="Aviso logo" width="320" />
  <img class="logo-dark"  src="./images/logo_dark.svg"  alt="Aviso logo" width="320" />
</div>

# Introduction

aviso is ECMWF's notification system for data-driven workflows. It runs as a
client and a server: [aviso-server](https://github.com/ecmwf/aviso-server)
tracks streams of events and pushes them out in close to real time; clients
connect, subscribe to the streams they care about, and react when a dataset they
were waiting for has landed.

This site documents the **client** side. This repo holds three clients: a
command-line tool, a Rust library, and a Python package. Pick the one that fits
how you work.

## From the command line

The `aviso` binary publishes notifications, listens for new ones, and replays
history. It runs on Linux and macOS.

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

Start with the [CLI overview](./cli/overview.md).

## From Python

The `pyaviso` package wraps the Rust core through PyO3, so you get a real Python
API: typed value objects, structured exceptions, `for`-iteration over a watch
stream, and the same trigger surface the CLI uses.

```python
import os
import pyaviso

client = pyaviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())

for notification in client.listen("mars", filter={"class": "od"}):
    print(notification.sequence, notification.payload)
```

Start with the [Python overview](./python/overview.md).

## From a Rust program

The Rust library powers the CLI and is on crates.io. Use it when you want
notifications inside a Rust binary or service.

```rust,ignore
use aviso::AvisoClient;

let client = AvisoClient::builder()
    .base_url("https://aviso.example")
    .build()?;
```

Start with the [library guide](./developers/lib-guide.md).

## New here?

- [What aviso does](./getting-started/overview.md) and how the pieces fit
  together.
- [Install](./getting-started/install.md) the CLI or the library.
- [Quickstart](./getting-started/quickstart.md): your first publish and your
  first listener.

## Reference

- [CLI flags](./reference/cli-flags.md): every flag, every subcommand.
- [Listener YAML](./reference/listener-yaml.md): the file format the CLI reads.
- [State file](./reference/state-file.md): what aviso writes to disk to remember
  where it left off.
- [Rust API](./reference/rust-api.md): types, traits, and functions.
