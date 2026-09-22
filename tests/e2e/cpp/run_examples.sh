#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

# Runs every C++ example against the real e2e stack.
#
# Examples that only make requests run directly. Examples that listen are
# driven: started in the background, fed notifications with 02_publish until
# they stop on their own, and failed if they have not stopped after a bounded
# number of publishes. Each listener runs in its own scratch directory so the
# files some of them write (logs, the resume position) do not leak between
# runs or into the checkout.

set -euo pipefail

BUILD_DIR="$(cd "${BUILD_DIR:-build/cpp}" && pwd)"
export AVISO_BASE_URL="${AVISO_BASE_URL:-http://aviso-server:8000}"
export AVISO_USERNAME="${AVISO_USERNAME:-producer-user}"
export AVISO_PASSWORD="${AVISO_PASSWORD:-producer-pass}"
# The examples read the aviso config file first. Point it and the
# credentials file at nothing so the run does not depend on the host.
export AVISO_CLIENT_CONFIG_FILE="${AVISO_CLIENT_CONFIG_FILE:-/nonexistent/aviso-e2e/config.yaml}"
export AVISO_CREDENTIALS_FILE="${AVISO_CREDENTIALS_FILE:-/nonexistent/aviso-e2e/credentials.yaml}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"

# Request-only examples: run, and the exit code is the verdict.
for name in 01_schema 02_publish 04_publish_many 06_publish_polygon 03_error_handling 01_basic 02_fan_out; do
  echo "== $name =="
  "$BUILD_DIR/$name"
done

# Replay-only reads history and ends by itself. Something must be there to
# read, which the publishes above guarantee.
echo "== 02_replay_only =="
"$BUILD_DIR/02_replay_only"

drive() {
  local name="$1"
  local out="$work/$name.out"
  "$BUILD_DIR/$name" >"$out" 2>&1 &
  local pid=$!
  local i
  for i in $(seq 1 40); do
    kill -0 "$pid" 2>/dev/null || break
    "$BUILD_DIR/02_publish" >/dev/null 2>&1 || true
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

# 05_filter wants the date 02_publish sends, so driving it is enough.
# 04_stop_from_outside ends on its own timer; the publishes give it something
# to print meanwhile.
for name in 03_listen 05_filter 04_stop_from_outside 01_echo 02_log 03_command 04_webhook 05_multiple; do
  echo "== $name =="
  drive "$name"
done

# A listener whose watch fails must say so through its exit code. Without
# this check a handler that ignored on_end() would pass every run above.
echo "== 03_listen with a rejected credential (must fail) =="
rc=0
AVISO_PASSWORD=wrong timeout 30 "$BUILD_DIR/03_listen" || rc=$?
# 124 is timeout's own status for a listener that never ended; 125 and up
# are timeout's infrastructure failures. Only the listener's own non-zero
# exit is the outcome this check wants.
if [ "$rc" -eq 0 ] || [ "$rc" -ge 124 ]; then
  echo "FAIL: a rejected credential exited $rc" >&2
  exit 1
fi

# Resume is the one that needs two runs: the second must pick up after the
# position the first saved, which is what the file in $work carries across.
echo "== 01_resume_from_sequence (first run) =="
drive 01_resume_from_sequence
test -s resume.seq || { echo "FAIL: no position saved" >&2; exit 1; }
echo "== 01_resume_from_sequence (second run, resuming) =="
drive 01_resume_from_sequence | tee resume2.out
grep -q '^resuming after #' resume2.out || { echo "FAIL: second run did not resume" >&2; exit 1; }

echo "C++ examples e2e: OK"
