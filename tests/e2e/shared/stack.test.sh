#!/usr/bin/env bash
#
# Offline tests for stack.sh readiness polling. The helpers are sourced and
# `curl` and `docker` are shadowed by shell functions, so no real stack or
# network is needed. They cover the three outcomes that matter for the e2e
# merge gate: ready once the endpoint answers, fail fast on an exited
# container, and time out when a running container never answers.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tests/e2e/shared/stack.sh
source "$here/stack.sh"

# Mocks shadowing the real binaries. CURL_OK_AFTER counts curl calls down to a
# success (negative = never); MOCK_STATUS is the container state docker reports.
CURL_OK_AFTER=-1
MOCK_STATUS="running"

curl() {
  if [ "$CURL_OK_AFTER" -ge 0 ]; then
    CURL_OK_AFTER=$((CURL_OK_AFTER - 1))
    [ "$CURL_OK_AFTER" -lt 0 ] && return 0
  fi
  return 1
}

docker() {
  case "$*" in
  "compose ps -a -q "*) echo "fake-container-id" ;; # -a required to see exited
  "inspect -f "*) echo "$MOCK_STATUS" ;;
  *) : ;; # compose ps -a / logs from dump_diagnostics: no-op
  esac
}

passed=0
failed=0

ready_case() {
  CURL_OK_AFTER=2
  MOCK_STATUS="running"
  TIMEOUT=10
  poll_endpoint nats "http://test/healthz"
}

timeout_case() {
  CURL_OK_AFTER=-1
  MOCK_STATUS="running"
  TIMEOUT=1
  poll_endpoint nats "http://test/healthz"
}

failfast_case() {
  CURL_OK_AFTER=-1
  MOCK_STATUS="exited"
  TIMEOUT=60
  poll_endpoint nats "http://test/healthz"
}

check() { # name expected_rc fn
  local name="$1" expected="$2" fn="$3" rc=0
  "$fn" >/dev/null 2>&1 || rc=$?
  if { [ "$expected" = 0 ] && [ "$rc" -eq 0 ]; } ||
    { [ "$expected" = 1 ] && [ "$rc" -ne 0 ]; }; then
    echo "ok   - $name"
    passed=$((passed + 1))
  else
    echo "FAIL - $name (expected rc class $expected, got $rc)"
    failed=$((failed + 1))
  fi
}

check "ready: succeeds once the endpoint answers" 0 ready_case
check "timeout: fails when a running container never answers" 1 timeout_case

# Fail-fast: an exited container must fail well under its 60s TIMEOUT, not wait
# it out. A broken fail-fast would take ~60s and trip the elapsed bound.
start=$(date +%s)
rc=0
failfast_case >/dev/null 2>&1 || rc=$?
elapsed=$(($(date +%s) - start))
if [ "$rc" -ne 0 ] && [ "$elapsed" -lt 10 ]; then
  echo "ok   - fail-fast: gives up promptly on an exited container (${elapsed}s)"
  passed=$((passed + 1))
else
  echo "FAIL - fail-fast (rc=$rc, elapsed=${elapsed}s, expected fail under 10s)"
  failed=$((failed + 1))
fi

echo "----"
echo "passed: $passed  failed: $failed"
[ "$failed" -eq 0 ]
