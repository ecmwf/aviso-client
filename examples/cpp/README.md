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

## Examples

- `schema_smoke.cpp`: prints the library version, builds a client, and calls the
  blocking `schema()` verb. With no argument it targets an unreachable URL and
  catches the resulting `aviso::Error`, so it runs without a server. Pass a
  reachable base URL to fetch a real schema:

  ```bash
  ./build/cpp/schema_smoke http://localhost:8000
  ```

- `publish.cpp`: publishes a notification with `notify()` and then reads the
  stream's schema with `schema_for()`. It needs a running server and, for an
  auth-required stream, credentials. Against the [`tests/e2e`](../../tests/e2e)
  stack, publish to `test_event` as the producer account:

  ```bash
  ./build/cpp/publish http://localhost:8000 producer-user producer-pass
  ```

- `watch.cpp`: starts a callback `watch()` and, from another thread on the same
  client, publishes a few notifications so the live watch receives them. It
  prints each one and stops after a target count. Against the `tests/e2e` stack,
  watch and publish to `test_event` as the producer account, stopping after 3:

  ```bash
  ./build/cpp/watch http://localhost:8000 producer-user producer-pass test_event 3
  ```
