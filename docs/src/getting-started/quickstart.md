# Quickstart

The fastest path from nothing to a working notification. Pick the surface you
have, follow the four steps, and you are done.

## 1. Get the binary

```bash
pip install pyaviso
aviso --version
```

The Python package bundles the `aviso` command-line tool, so pip is all you
need. If you prefer a Rust-native install, `cargo install aviso-cli` gives you
the same command. The full install guide is at [Install](./install.md).

## 2. Point at a server

Tell aviso where the server is and how to authenticate. The simplest way is
environment variables:

```bash
export AVISO_BASE_URL=https://aviso.example
export AVISO_TOKEN=your-bearer-token
```

Or pass them on the command line each time, with `--base-url` and `--token`.

If you would rather keep them in a file, see
[CLI configuration](../cli/configuration.md).

## 3. Discover notification types

First, see which notification types the server offers:

```bash
aviso schema list
```

For a server configured with just the example `mars` type, the terminal output
is:

```text
1 schema(s) registered (status: success)
- mars
```

This lists registered notification types, not stored notifications or data
files. Pick a type from your server's list and inspect its schema to see which
identifiers you can filter on:

```bash
aviso schema get mars
```

Example output from a server with a minimal `mars` schema:

```json
{
  "event_type": "mars",
  "schema": {
    "identifier": {
      "class": {
        "description": "MARS class.",
        "required": true,
        "type": "EnumHandler",
        "values": [
          "od",
          "rd"
        ]
      }
    },
    "payload": {
      "required": false
    }
  },
  "status": "success"
}
```

Here, `class` is required in the listener's filter. `EnumHandler` means its
value must come from the listed values, `od` or `rd`. We will choose `od` below.
The payload is optional for notifications of this type.

This JSON describes the schema already registered on the server; it is command
output, not a configuration file to install. Your server may offer other types
or require more identifiers. Use its schema to choose the event type and supply
all required identifiers in the examples below.

## 4. Listen for something

For the example schema above, listen for `mars` notifications with `class=od`:

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

Press Ctrl+C to stop.

Every matching notification prints to your terminal as JSON. When you redirect
the output to a file or pipe it into another tool, the format changes to one
compact JSON object per line so you can chain it with `jq`:

```bash
aviso listen --event mars --identifiers '{"class":"od"}' | jq -r '.payload'
```

That is it. You have a working listener.

## What just happened

- aviso connected to the server and opened a long-lived stream.
- It asked for `mars` events with `class=od`.
- It echoed each match to your terminal.
- It remembered the last notification it printed in
  `~/.config/aviso/state.json`, so the next run starts after that point.

Re-running the same command resumes from where you left off. A notification can
still be redelivered after a crash or failed checkpoint, so production triggers
should be safe to run more than once. To start fresh next time, add
`--no-state-store`.

## Calling aviso from Python

The same listener as step 4, through the native Python API. The
`pip install pyaviso` from step 1 already gave you the `pyaviso` package, and
`pyaviso.Env()` reads the same environment variables you exported in step 2:

```python
"""Listen for mars notifications and print each one as it arrives."""

import os
import pyaviso

client = pyaviso.AvisoClient(base_url=os.environ["AVISO_BASE_URL"], auth=pyaviso.Env())

for notification in client.listen("mars", filter={"class": "od"}):
    print(f"seq={notification.sequence} payload={notification.payload}")
```

Notifications arrive as typed objects with `sequence`, `identifier`, and
`payload` fields, so there is no JSON to re-parse. Publishing, resuming across
restarts, async, and error handling are covered in the
[Python section](../python/overview.md), starting with its
[quickstart](../python/quickstart.md).

## Calling aviso from a Rust program

```rust,ignore
use std::collections::BTreeMap;
use aviso::{
    watch::{Trigger, WatchRequest},
    AvisoClient,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = AvisoClient::builder()
        .base_url("https://aviso.example")
        .build()?;

    let mut filter = BTreeMap::new();
    filter.insert("class".to_string(), serde_json::json!("od"));

    let req = WatchRequest::watch("mars")
        .with_filter(filter)
        .with_triggers(vec![Trigger::echo()]);

    let mut stream = client.watch(req)?;
    while let Some(notification) = stream.recv().await {
        let n = notification?;
        println!("seq {}: {}", n.sequence, n.payload);
    }
    Ok(())
}
```

The filter must include any identifier the event type's schema marks
`required: true` (run `aviso schema get <TYPE>` to see which).

The full library walkthrough is in the
[library guide](../developers/lib-guide.md).

## Next

- [Publish a notification](../cli/publish-and-listen.md#publish) (your first
  `aviso notify`).
- [Listen with a YAML file](../cli/publish-and-listen.md#listen-with-a-yaml-file):
  named listeners, multiple triggers, the configuration you keep around.
- [Concepts](./concepts.md): the five ideas you need to get fluent.
