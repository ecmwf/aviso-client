# Install

Most users need exactly one command:

```bash
pip install pyaviso
aviso --version
```

The Python package carries both surfaces: the importable `pyaviso` library
and the `aviso` command-line tool. The wheel installs the CLI as a console
script that runs the Rust core in-process, so there is no Rust toolchain to
set up and no separate binary to download.

| You want to | Install |
|---|---|
| Call aviso from Python | [`pip install pyaviso`](../python/install.md) |
| Run the `aviso` command-line tool | `pip install pyaviso` (the CLI is bundled), or [`cargo install aviso-cli`](../cli/install.md) for a Rust-native binary |
| Use the Rust library in your own crate | `cargo add aviso` |

## Command line without Python

If you do not use Python, cargo builds the same CLI from source:

```bash
cargo install aviso-cli
aviso --version
```

`cargo install` puts the binary in `~/.cargo/bin/aviso`, which is on your PATH
if you installed Rust through [rustup](https://rustup.rs/). Both install paths
give you the same `aviso` command and behaviour.

The [CLI install page](../cli/install.md) covers the from-source path and how to
verify your install.

## Rust library, in your `Cargo.toml`

```toml
[dependencies]
aviso = "2.0"
tokio = { version = "1.53", features = ["macros", "rt-multi-thread"] }
serde_json = "1.0"
```

aviso is async and runs on [tokio](https://tokio.rs/). The full library guide is
in the [developers section](../developers/lib-guide.md).

## What you need on the server side

Whichever surface you pick, you will need:

- The base URL of an aviso-server you can talk to (your team or ECMWF gives you
  this).
- Credentials, when the server requires them (usually a bearer token).

See [authentication providers](../concepts/auth-providers.md) for the five ways
to attach credentials.
