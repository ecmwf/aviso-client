<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# C++ end-to-end checks

[`run_examples.sh`](./run_examples.sh) runs the already-built C++ examples
([`examples/cpp`](../../../examples/cpp)) against a live `aviso-server` and
checks their results. The CI `e2e` job runs it inside the CI image on the
compose network, after the Python and Rust suites.

It runs the request-only examples (`01_schema`, `02_publish`, `04_publish_many`,
`06_publish_polygon`, `03_error_handling`, the two `async/` ones, and
`02_replay_only`) directly, and drives every listening example by publishing
repeatedly until it exits on its own; the output of `07_watch_many` must contain
notifications from both of its watches. The resume example runs twice, and the
second run must report that it resumed after the position the first one saved.
One listener runs with a rejected credential and must exit non-zero, which
proves a failed watch is visible in the exit code and not only on stderr.
Settings come from the environment: `AVISO_BASE_URL`, `AVISO_USERNAME`,
`AVISO_PASSWORD` (defaults: the e2e stack's `aviso-server` and the producer
account) and `BUILD_DIR` (default `build/cpp`). `AVISO_TOKEN` is cleared and the
aviso config and credentials files are pointed at nothing, so a run does not
depend on what the host has in its environment or in `~/.config/aviso`.

Run it locally against the stack:

```bash
bash tests/e2e/shared/stack.sh up
cargo build -p aviso-ffi
cmake -S examples/cpp -B build/cpp -DAVISO_FFI_LIB_DIR="$PWD/target/debug"
cmake --build build/cpp
AVISO_BASE_URL=http://localhost:8000 bash tests/e2e/cpp/run_examples.sh
bash tests/e2e/shared/stack.sh down
```
