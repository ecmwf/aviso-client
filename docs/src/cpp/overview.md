# C++ binding

aviso ships a C++ binding as a stable C ABI plus a header-only C++ facade over
it. The `aviso-ffi` crate builds a `libaviso_ffi` library and generates a C
header (`aviso.h`); the hand-written `aviso.hpp` adds RAII handles,
`std::string` and `std::optional` ergonomics, and a throwing `aviso::Error`.

The point of the C ABI is packaging. A C++ application links the prebuilt
library and compiles the facade with its own toolchain, so the binding works
under C++17 and needs no Rust toolchain in the consumer's build.

## What you can do today

- Build a client from a base URL, optionally with HTTP Basic credentials.
- Publish notifications with `notify`; see [Publishing](./publish.md).
- Watch a stream with a callback handler, with filtering and replay; see
  [Watching](./watch.md).
- Attach triggers (echo, log, command, webhook, Teams, post) to a watch; see
  [Triggers](./triggers.md).
- Read schemas with `schema` and `schema_for`, and run the operator-only admin
  calls `wipe_stream`, `wipe_all`, and `delete_notification`; see
  [Operations](./operations.md).
- Read structured errors: every failure throws an `aviso::Error` whose `what()`
  is a human-readable message and whose `error()` returns the kind, HTTP status,
  and request id.

An async form of the verbs lands as the binding grows.

## A first call

```cpp
#include "aviso.hpp"
#include <iostream>

int main() {
  try {
    aviso::Client client = aviso::ClientBuilder("http://localhost:8000")
                               .basic_auth("user", "pass")
                               .build();
    std::cout << client.schema() << '\n';
  } catch (const aviso::Error& error) {
    std::cerr << "aviso error: " << error.what() << '\n';
    return 1;
  }
}
```

## How calls behave

Every call blocks until the server responds and throws `aviso::Error` on any
failure, so wrap them in `try` / `catch`. JSON-shaped data crosses as strings:
`notify` takes its payload as a JSON string and the calls return their responses
as JSON strings, which you parse with whatever JSON library your application
already uses.

These calls must not run inside a watch or async callback (a runtime thread);
doing so throws an `aviso::Error` with kind `AvisoErrorKind_InvalidUsage` rather
than deadlocking. See [Watching](./watch.md) for the callback surface.

## Building against the library

The worked, tested consumer lives in
[`examples/cpp`](https://github.com/ecmwf/aviso-client/tree/main/examples/cpp).
It links the library and compiles the facade through CMake. For local
development, build the library from the workspace first, then point CMake at the
headers and the build directory:

```bash
cargo build -p aviso-ffi
cmake -S examples/cpp -B build/cpp
cmake --build build/cpp
./build/cpp/schema_smoke            # no server: catches and prints the error
./build/cpp/schema_smoke http://localhost:8000
```

The two CMake cache variables `AVISO_FFI_INCLUDE_DIR` and `AVISO_FFI_LIB_DIR`
default to the in-tree locations; point them at a prebuilt drop to build against
shipped artifacts with no Rust toolchain.

## The C header and the facade

`aviso.h` is generated from the Rust surface by `cbindgen` and is the source of
truth for the ABI; `aviso.hpp` is hand-written over it and is the recommended
surface for C++ callers. C callers can use `aviso.h` directly: every fallible
call returns an owning `AvisoOutcome` that you inspect, take a value out of, and
free.
