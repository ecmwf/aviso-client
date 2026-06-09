#!/usr/bin/env bash
#
# Unit tests for publish-dry-run.sh.
#
# `cargo` is faked with a shim that records the order of publish calls and
# fails selected crates with configurable stderr, so the classification logic
# (finesse always fatal, first-publish resolution gap tolerated, anything else
# fatal) is exercised offline.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$root/scripts/publish-dry-run.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Fake `cargo publish --dry-run -p <crate>`: append the crate to the call log;
# if FAIL_SPEC contains "<crate>=<mode>", emit that mode's error text and fail.
mkdir -p "$tmp/bin"
cat >"$tmp/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
crate=""
prev=""
for a in "$@"; do
  [ "$prev" = "-p" ] && crate="$a"
  prev="$a"
done
echo "$crate" >>"$CALL_LOG"
mode=""
for spec in ${FAIL_SPEC:-}; do
  case "$spec" in
  "$crate"=*) mode="${spec#*=}" ;;
  esac
done
case "$mode" in
"") echo "ok: packaged $crate" ;;
unresolved)
  echo "error: no matching package named \`aviso\` found" >&2
  exit 101
  ;;
unresolved-ffi)
  echo "error: no matching package named \`aviso-ffi\` found" >&2
  exit 101
  ;;
unresolved-req)
  echo "error: failed to select a version for the requirement \`finesse = \"^2\"\`" >&2
  exit 101
  ;;
unresolved-external)
  echo "error: no matching package named \`serde\` found" >&2
  exit 101
  ;;
metadata)
  echo "error: missing field description" >&2
  exit 101
  ;;
*)
  echo "fake cargo: unknown FAIL_SPEC mode '$mode'" >&2
  exit 2
  ;;
esac
EOF
chmod +x "$tmp/bin/cargo"

passed=0
failed=0

check() { # name expected_rc fail_spec
  local name="$1" expected="$2" spec="$3"
  local rc=0
  CALL_LOG="$tmp/calls.$RANDOM" FAIL_SPEC="$spec" PATH="$tmp/bin:$PATH" \
    bash "$script" >/dev/null 2>&1 || rc=$?
  if { [ "$expected" = "0" ] && [ "$rc" -eq 0 ]; } ||
    { [ "$expected" = "1" ] && [ "$rc" -ne 0 ]; }; then
    echo "ok   - $name"
    passed=$((passed + 1))
  else
    echo "FAIL - $name (expected rc class $expected, got $rc)"
    failed=$((failed + 1))
  fi
}

check "passes when every crate dry-runs clean" 0 ""
check "tolerates the resolution gap on a dependent crate" 0 "aviso=unresolved"
check "tolerates the requirement form of the gap" 0 "aviso-cli=unresolved-req"
check "tolerates the gap on several dependents" 0 "aviso=unresolved aviso-cli=unresolved aviso-ffi=unresolved"
check "tolerates the gap when it names aviso-ffi" 0 "aviso-ffi=unresolved-ffi"
check "fails when finesse fails, whatever the reason" 1 "finesse=unresolved"
check "fails a dependent crate on a non-gap error" 1 "aviso=metadata"
check "fails when the unresolved package is external" 1 "aviso=unresolved-external"

# Order contract: the crates must be dry-run in publish order.
log="$tmp/order.log"
CALL_LOG="$log" FAIL_SPEC="" PATH="$tmp/bin:$PATH" bash "$script" >/dev/null 2>&1
if [ "$(paste -sd' ' "$log")" = "finesse aviso aviso-cli aviso-ffi" ]; then
  echo "ok   - dry-runs the crates in publish order"
  passed=$((passed + 1))
else
  echo "FAIL - dry-runs the crates in publish order (got: $(paste -sd' ' "$log"))"
  failed=$((failed + 1))
fi

# A tolerated gap must not stop the remaining crates from being checked.
log="$tmp/continue.log"
CALL_LOG="$log" FAIL_SPEC="aviso=unresolved" PATH="$tmp/bin:$PATH" \
  bash "$script" >/dev/null 2>&1
if [ "$(paste -sd' ' "$log")" = "finesse aviso aviso-cli aviso-ffi" ]; then
  echo "ok   - continues past a tolerated gap"
  passed=$((passed + 1))
else
  echo "FAIL - continues past a tolerated gap (got: $(paste -sd' ' "$log"))"
  failed=$((failed + 1))
fi

echo
echo "passed=$passed failed=$failed"
[ "$failed" -eq 0 ]
