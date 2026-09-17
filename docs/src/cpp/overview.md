<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

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
- Listen to a stream with a callback handler, with filtering and replay; see
  [Listening](./watch.md).
- Attach triggers (echo, log, command, webhook, Teams, post) to a listener; see
  [Triggers](./triggers.md).
- Read schemas with `schema` and `schema_for`, and run the operator-only admin
  calls `wipe_stream`, `wipe_all`, and `delete_notification`; see
  [Operations](./operations.md).
- Call any verb asynchronously: each has a `*_async` form returning a
  `std::future`; see [Async](./async.md).
- Read structured errors: every failure throws an `aviso::Error` whose `what()`
  is a human-readable message and whose `error()` returns the kind, HTTP status,
  and request id.

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

These calls must not run inside a listener or async callback (a runtime thread);
doing so throws an `aviso::Error` with kind `AvisoErrorKind_InvalidUsage` rather
than deadlocking. See [Listening](./watch.md) for the callback surface.

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
./build/cpp/schema_smoke                          # no server: catches the error
AVISO_BASE_URL=http://localhost:8000 ./build/cpp/schema_smoke
```

The examples read their connection settings (`AVISO_BASE_URL`, and the optional
`AVISO_USERNAME` / `AVISO_PASSWORD`) from the environment, so credentials never
appear on the command line. The two CMake cache variables
`AVISO_FFI_INCLUDE_DIR` and `AVISO_FFI_LIB_DIR` default to the in-tree
locations; point them at a prebuilt drop to build against shipped artifacts with
no Rust toolchain.

## Installing with cargo-c

For a standard system install rather than an in-tree build,
[cargo-c](https://github.com/lu-zero/cargo-c) drops the library, a pkg-config
`.pc`, and the headers in the usual layout:

```bash
cargo install cargo-c   # once
cargo cinstall -p aviso-ffi --release --prefix=/usr/local --libdir=/usr/local/lib
```

A consumer then discovers it through pkg-config, with no in-tree paths and no
Rust toolchain:

```cmake
find_package(PkgConfig REQUIRED)
pkg_check_modules(AVISO_FFI REQUIRED IMPORTED_TARGET aviso_ffi)
target_link_libraries(my_app PRIVATE PkgConfig::AVISO_FFI)
```

The install places the headers under `include/aviso_ffi/`, and the `.pc` puts
that directory on the include path, so `#include "aviso.hpp"` works unchanged.

## The C header and the facade

`aviso.h` is generated from the Rust surface by `cbindgen` and is the source of
truth for the ABI; `aviso.hpp` is hand-written over it and is the recommended
surface for C++ callers. C callers can use `aviso.h` directly: every fallible
call returns an owning `AvisoOutcome` that you inspect, take a value out of, and
free.
