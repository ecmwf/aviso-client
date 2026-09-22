<!--
SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
SPDX-License-Identifier: Apache-2.0
-->

# C++ examples

Runnable programs that show what the C++ binding does, one scenario per file.
Each one links the prebuilt library and compiles `aviso.hpp` with your own
toolchain, the way your program would. CI builds all of them with `-Werror` and
runs them against a real server, so they are also the binding's tests.

## Build

The library must exist first. From a checkout:

```bash
cargo build -p aviso-ffi
cmake -S examples/cpp -B build/cpp
cmake --build build/cpp
```

CMake defaults to the in-tree headers and `target/debug`. To build against a
prebuilt drop instead, with no Rust toolchain, point it at the drop's
directories:

```bash
cmake -S examples/cpp -B build/cpp \
  -DAVISO_FFI_INCLUDE_DIR=/path/to/drop/include \
  -DAVISO_FFI_LIB_DIR=/path/to/drop/lib
cmake --build build/cpp
```

Every example becomes an executable named after its file, so
`./build/cpp/02_publish` runs `basics/02_publish.cpp`.

## Connect

Every example connects through [`common.hpp`](./common.hpp), which reads the
aviso config file first and lets the environment override it. If you already
use the `aviso` command on this machine, the examples work with no setup.
Otherwise set:

```bash
export AVISO_BASE_URL=http://localhost:8000
export AVISO_USERNAME=producer-user
export AVISO_PASSWORD=producer-pass
```

Those are the producer account of the local e2e stack, which is the easiest
server to try against. It ships the `test_event` stream the examples use, with
`date` and `time` identifiers:

```bash
bash tests/e2e/shared/stack.sh up      # auth-o-tron + aviso-server + NATS
./build/cpp/01_schema                  # should print the catalog
bash tests/e2e/shared/stack.sh down    # when done
```

A bearer token works too: set `AVISO_TOKEN` instead of the username and
password. Against your own server, change `kEventType` in `common.hpp` to a
stream that exists there and adjust the identifier fields in the publishing
examples to match; the shape of every call stays the same.

`common.hpp` also carries two small helpers the listeners share. `Handler`
remembers the error `on_end()` receives, and `finish()` waits for the watch and
turns that error into the exit code. Without them a failed watch would look like
a clean exit, which is the one mistake every first listener makes.

## What is here

The directories group by purpose, not by API surface. Read the comment at the
top of each file first: it says what the example shows and what you should see
when you run it.

### `basics/`: the calls everyone needs first

| File | Shows |
|---|---|
| `01_schema.cpp` | Ask the server what streams it has and what fields they take. Exits cleanly with no server configured or reachable, so it is also the build check. |
| `02_publish.cpp` | Publish one notification with an identifier and a payload. |
| `03_listen.cpp` | Listen to a stream with a handler; stop after three. |
| `04_publish_many.cpp` | Publish a batch concurrently; see a per-item failure reported instead of thrown. |
| `05_filter.cpp` | Receive only the notifications whose identifier matches a filter. |
| `06_publish_polygon.cpp` | Publish a spatial identifier and a required payload with `notify_json()`. |

### `resilience/`: the patterns you need on top of listen

| File | Shows |
|---|---|
| `01_resume_from_sequence.cpp` | Save the last sequence to a file, resume after it on the next run, and get the missed notifications first with no gap. Run it twice. |
| `02_replay_only.cpp` | Read history since a date and stop, continuing past the server's replay cap until it has everything. |
| `03_error_handling.cpp` | What each error kind looks like and which ones are worth a retry. |
| `04_stop_from_outside.cpp` | End a listener from a signal handler or a timer with `Watch::stop()`, cleanly. |

### `triggers/`: do something for every notification

| File | Shows |
|---|---|
| `01_echo.cpp` | Print each notification as JSON. The one to try first. |
| `02_log.cpp` | Append each notification to a file. |
| `03_command.cpp` | Run a shell command with the notification in `AVISO_*` environment variables. |
| `04_webhook.cpp` | POST each notification to a URL with a templated body. Opens its own tiny receiver so it runs without one. |
| `05_multiple.cpp` | Several triggers on one watch, and what `required` means when one fails. |

`teams()` and `post()` have the same shape as `webhook()`; see the
[trigger reference](../../docs/src/triggers/overview.md) for their details.

### `async/`: the same verbs, returning `std::future`

| File | Shows |
|---|---|
| `01_basic.cpp` | Two requests in flight at once. |
| `02_fan_out.cpp` | Start ten requests, then collect them, in about one round trip. |

## Listening examples stop on their own

Every listener stops after three notifications so you never have to Ctrl+C.
The one exception is `resilience/04_stop_from_outside.cpp`, which is about
stopping from outside and so ends on Ctrl+C or its own timer instead. Run a
listener in one terminal and publish from another:

```bash
./build/cpp/03_listen              # waits
./build/cpp/02_publish             # in a second terminal, three times
```

The stop-after-three is for following along. In real code the handler returns
`true` for as long as you want to keep listening, and something outside calls
`Watch::stop()` when it is time to go. `resilience/04_stop_from_outside.cpp`
shows that with Ctrl+C and a timer.

## Testing

[`tests/e2e/cpp/run_examples.sh`](../../tests/e2e/cpp/run_examples.sh) runs
all seventeen against the e2e stack. The request-only ones run directly. The
listeners are driven by publishes until they stop. The resume example runs
twice to check that the second run picks up where the first stopped, and one
listener runs with a bad credential to check that a failed watch exits 1. CI
runs it on every change.
