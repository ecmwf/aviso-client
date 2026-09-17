<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# C++ end-to-end checks

[`run_examples.sh`](./run_examples.sh) runs the already-built C++ examples
([`examples/cpp`](../../../examples/cpp)) against a live `aviso-server` and
checks their results. The CI `e2e` job runs it inside the CI image on the
compose network, after the Python and Rust suites.

It runs `schema_smoke`, `publish`, and `async` directly, and drives the
watch-only `watch` and `trigger` examples by publishing repeatedly until each
collects enough notifications and exits on its own. Settings come from the
environment: `AVISO_BASE_URL`, `AVISO_USERNAME`, `AVISO_PASSWORD` (defaults: the
e2e stack's `aviso-server` and the producer account) and `BUILD_DIR` (default
`build/cpp`).

Run it locally against the stack:

```bash
bash tests/e2e/shared/stack.sh up
cargo build -p aviso-ffi
cmake -S examples/cpp -B build/cpp -DAVISO_FFI_LIB_DIR="$PWD/target/debug"
cmake --build build/cpp
AVISO_BASE_URL=http://localhost:8000 bash tests/e2e/cpp/run_examples.sh
bash tests/e2e/shared/stack.sh down
```
