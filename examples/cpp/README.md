# C++ examples

Worked C++ consumers of the aviso C ABI and its header-only facade
([`crates/aviso-ffi`](../../crates/aviso-ffi)). Each example links the prebuilt
library and compiles the facade with the system C++ toolchain, the same way a
real consumer would, so these double as the binding's tested reference.

## Building

The library must exist first. For local development, build it from the
workspace:

```bash
cargo build -p aviso-ffi
```

Then configure and build the examples, pointing CMake at the headers and the
library:

```bash
cmake -S examples/cpp -B build/cpp \
  -DAVISO_FFI_INCLUDE_DIR="$PWD/crates/aviso-ffi/include" \
  -DAVISO_FFI_LIB_DIR="$PWD/target/debug"
cmake --build build/cpp
```

Both `-D` paths default to the in-tree locations above, so on a normal checkout
`cmake -S examples/cpp -B build/cpp` is enough. Point them at a prebuilt drop
(its `include/` and library directory) to build against shipped artifacts with
no Rust toolchain.

## Configuration

Every example reads its connection settings from the environment
([`aviso_env.hpp`](./aviso_env.hpp)), so credentials never appear on the command
line:

| Variable | Meaning | Default |
|---|---|---|
| `AVISO_BASE_URL` | Server base URL | `http://localhost:8000` |
| `AVISO_USERNAME` | HTTP Basic username (optional) | unset |
| `AVISO_PASSWORD` | HTTP Basic password (optional) | unset |

Against the [`tests/e2e`](../../tests/e2e) stack, export the producer account
once and run any example:

```bash
export AVISO_BASE_URL=http://localhost:8000
export AVISO_USERNAME=producer-user
export AVISO_PASSWORD=producer-pass
```

## Examples

- `schema_smoke.cpp`: prints the version, connects, and calls `schema()`. With
  no server reachable it catches the transport error and exits cleanly, so it
  runs (and is exercised by CI) without a server.

  ```bash
  ./build/cpp/schema_smoke
  ```

- `publish.cpp`: publishes a notification with `notify()` and reads the stream's
  schema with `schema_for()`. The stream and identifier are declared at the top
  of the file. Against an auth-required stream, set the producer account above.

  ```bash
  ./build/cpp/publish
  ```

- `watch.cpp`: watches a stream (declared at the top of the file) and prints
  each notification, stopping after a fixed count. It only watches; publish to
  the stream from another terminal to see notifications:

  ```bash
  ./build/cpp/watch      # in one terminal
  ./build/cpp/publish    # in another, a few times
  ```

- `trigger.cpp`: watches a stream with a `log` trigger attached, so the trigger
  appends each notification to a file as the watch runs, then prints the file.
  Drive it by publishing from another terminal as with `watch`.

  ```bash
  ./build/cpp/trigger
  ```

- `async.cpp`: fires two async verbs (`notify_async` and `schema_async`) at once
  and waits on their `std::future`s, showing the future-based async form.

  ```bash
  ./build/cpp/async
  ```
