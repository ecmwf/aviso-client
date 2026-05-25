# Install

Pick the row that matches what you want to do.

| You want to | Install |
|---|---|
| Run the `aviso` command-line tool | [`cargo install aviso-cli`](../cli/install.md) |
| Use the Rust library in your own crate | `cargo add aviso` |
| Call aviso from Python | Install the CLI above and run it via `subprocess`, or install the native [Python package](../python/overview.md) |

## Command line, in one shot

```bash
cargo install aviso-cli
aviso --version
```

`cargo install` puts the binary in `~/.cargo/bin/aviso`, which is on your PATH if you installed Rust through [rustup](https://rustup.rs/).

If you do not have Rust installed yet, get it from <https://rustup.rs/>. The script that page provides is the smoothest way; it handles every supported platform.

The [CLI install page](../cli/install.md) covers the from-source path and how to verify your install.

## Rust library, in your `Cargo.toml`

```toml
[dependencies]
aviso = "0.1"
tokio = { version = "1.45", features = ["macros", "rt-multi-thread"] }
serde_json = "1.0"
```

aviso is async and runs on [tokio](https://tokio.rs/). The full library guide is in the [developers section](../developers/lib-guide.md).

## What you need on the server side

Whichever surface you pick, you will need:

- The base URL of an aviso-server you can talk to (your team or ECMWF gives you this).
- Credentials, when the server requires them (usually a bearer token).

See [authentication providers](../concepts/auth-providers.md) for the five ways to attach credentials.
