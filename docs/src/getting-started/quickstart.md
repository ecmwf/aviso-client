# Quickstart

The fastest path from nothing to a working notification. Pick the surface you have, follow the three steps, and you are done.

## 1. Get the binary

```bash
cargo install aviso-cli
aviso --version
```

If `cargo` is not installed, get it from <https://rustup.rs/>. The full install guide is at [Install](./install.md).

## 2. Point at a server

Tell aviso where the server is and how to authenticate. The simplest way is environment variables:

```bash
export AVISO_BASE_URL=https://aviso.example
export AVISO_TOKEN=your-bearer-token
```

Or pass them on the command line each time, with `--base-url` and `--token`.

If you would rather keep them in a file, see [CLI configuration](../cli/configuration.md).

## 3. Listen for something

```bash
aviso listen --event mars --identifiers '{"class":"od"}'
```

Press Ctrl+C to stop.

Every matching notification prints to your terminal as JSON. When you redirect the output to a file or pipe it into another tool, the format changes to one compact JSON object per line so you can chain it with `jq`:

```bash
aviso listen --event mars --identifiers '{"class":"od"}' | jq -r '.payload'
```

That is it. You have a working listener.

## What just happened

- aviso connected to the server and opened a long-lived stream.
- It asked for `mars` events with `class=od`.
- It echoed each match to your terminal.
- It remembered the last notification it printed in `~/.config/aviso/state.json`, so the next run starts after that point.

Re-running the same command resumes from where you left off. A notification can still be redelivered after a crash or failed checkpoint, so production triggers should be safe to run more than once. To start fresh next time, add `--no-state-store`.

## Calling aviso from Python

```python
import json, subprocess

proc = subprocess.Popen(
    ["aviso", "listen",
     "--event", "mars",
     "--identifiers", '{"class":"od"}'],
    stdout=subprocess.PIPE, text=True,
)
for line in proc.stdout:
    notification = json.loads(line)
    print(notification["sequence"], notification["payload"])
```

The native Python client, which avoids the subprocess and gives you typed value objects, is documented at [Python overview](../python/overview.md).

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

The filter must include any identifier the event type's schema marks `required: true` (run `aviso schema get <TYPE>` to see which).

The full library walkthrough is in the [library guide](../developers/lib-guide.md).

## Next

- [Publish a notification](../cli/publish-and-listen.md#publish) (your first `aviso notify`).
- [Listen with a YAML file](../cli/publish-and-listen.md#listen-with-a-yaml-file): named listeners, multiple triggers, the configuration you keep around.
- [Concepts](./concepts.md): the five ideas you need to get fluent.
