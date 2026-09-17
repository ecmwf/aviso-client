<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# aviso-cli

Command-line client for [`aviso-server`](https://github.com/ecmwf/aviso-server), ECMWF's notification service for data-driven workflows. Installs the `aviso` binary.

Built on the [`aviso`](https://crates.io/crates/aviso) core library.

```bash
cargo install aviso-cli
aviso --help
```

The same `aviso` command is also available from the [`pyaviso`](https://github.com/ecmwf/aviso-client/tree/main/python) Python package: `pip install pyaviso` installs a console script that runs this CLI through the extension, putting `aviso` on your PATH without a Rust toolchain.

See the [workspace repository](https://github.com/ecmwf/aviso-client) for usage and architectural background.

## License

Apache-2.0. See [`LICENSE`](https://github.com/ecmwf/aviso-client/blob/main/LICENSE). Copyright 2026 ECMWF and individual contributors.
