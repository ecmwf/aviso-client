# aviso-cli

Command-line client for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows. Installs the `aviso` binary.

Built on the [`aviso`](https://crates.io/crates/aviso) core library.

```bash
cargo install aviso-cli
aviso --help
```

The same `aviso` binary is also bundled in the [`pyaviso`](https://github.com/ecmwf/aviso-client/tree/main/python) Python package, so `pip install pyaviso` puts it on your PATH too.

See the [workspace repository](https://github.com/ecmwf/aviso-client) for usage and architectural background.

## License

Apache-2.0. See [`LICENSE.txt`](https://github.com/ecmwf/aviso-client/blob/main/LICENSE.txt). Copyright 2026 ECMWF and individual contributors.
