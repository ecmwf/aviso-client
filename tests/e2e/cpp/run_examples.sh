#!/usr/bin/env bash
# Runs the C++ examples against a live aviso-server and checks their results.
#
# Expects the server reachable at AVISO_BASE_URL (default the e2e stack's
# aviso-server) with the producer account, and the examples already built in
# BUILD_DIR (default build/cpp). Used by the e2e CI job and runnable locally
# against `tests/e2e/shared/stack.sh up`:
#
#   cargo build -p aviso-ffi
#   cmake -S examples/cpp -B build/cpp -DAVISO_FFI_LIB_DIR="$PWD/target/debug"
#   cmake --build build/cpp
#   AVISO_BASE_URL=http://localhost:8000 bash tests/e2e/cpp/run_examples.sh
set -euo pipefail

BUILD_DIR="$(cd "${BUILD_DIR:-build/cpp}" && pwd)"
export AVISO_BASE_URL="${AVISO_BASE_URL:-http://aviso-server:8000}"
export AVISO_USERNAME="${AVISO_USERNAME:-producer-user}"
export AVISO_PASSWORD="${AVISO_PASSWORD:-producer-pass}"

# Run from a scratch directory so example output (the trigger's log file) does
# not land in the checkout; clean it up on exit.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

echo "== schema_smoke ==" && "$BUILD_DIR/schema_smoke"
echo "== publish ==" && "$BUILD_DIR/publish"
echo "== publish_many ==" && "$BUILD_DIR/publish_many"
echo "== async ==" && "$BUILD_DIR/async"

# Drives a watch-only example: start it, then publish repeatedly until it has
# collected enough notifications and exits on its own (it stops after a fixed
# count). Bounded so a missed connect cannot hang the job.
drive() {
  local name="$1"
  # Keep the output file under $work so the EXIT trap cleans it up.
  local out="$work/$name.out"
  "$BUILD_DIR/$name" >"$out" 2>&1 &
  local pid=$!
  local i
  for i in $(seq 1 40); do
    kill -0 "$pid" 2>/dev/null || break
    "$BUILD_DIR/publish" >/dev/null 2>&1 || true
    sleep 0.5
  done
  if kill -0 "$pid" 2>/dev/null; then
    # Bounded shutdown: SIGTERM, a short grace period, then SIGKILL, so a stuck
    # process cannot make the wait below hang.
    kill "$pid" 2>/dev/null || true
    local g
    for g in 1 2 3 4 5; do kill -0 "$pid" 2>/dev/null || break; sleep 0.2; done
    kill -9 "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    cat "$out"
    echo "FAIL: $name did not finish after 40 publishes" >&2
    return 1
  fi
  if wait "$pid"; then
    cat "$out"
  else
    local rc=$?
    cat "$out"
    echo "FAIL: $name exited $rc" >&2
    return "$rc"
  fi
}

echo "== watch ==" && drive watch
echo "== trigger ==" && drive trigger

echo "C++ examples e2e: OK"
