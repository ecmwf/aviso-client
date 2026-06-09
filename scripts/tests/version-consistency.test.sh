#!/usr/bin/env bash
#
# Unit tests for version-consistency.sh.
#
# A throwaway workspace tree is generated per case: a root manifest with a
# members array and a floating workspace dependency, one member inheriting the
# workspace version with an inline `=` pin, one member pinning through a
# multiline [dependencies.<name>] table, and one plain member. Each invariant
# is then checked offline, including the failure paths.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$root/scripts/version-consistency.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# make_tree <dir> <ws-version> <pin> <float-req>
# The external dependency (no `path`) must never be version-checked.
make_tree() {
  local dir="$1" ws="$2" pin="$3" float="$4"
  mkdir -p "$dir/crates/core" "$dir/crates/inline" "$dir/crates/multi"
  cat >"$dir/Cargo.toml" <<EOF
[workspace]
members = [
    "crates/core",
    "crates/inline",
    "crates/multi",
]

[workspace.package]
version = "$ws"

[workspace.dependencies]
external = "9.9"
core = { version = "$float", path = "crates/core" }
EOF
  cat >"$dir/crates/core/Cargo.toml" <<EOF
[package]
name = "core"
version.workspace = true
EOF
  cat >"$dir/crates/inline/Cargo.toml" <<EOF
[package]
name = "inline"
version.workspace = true

[dependencies]
external = "9.9"
core = { path = "../core", version = "$pin" }
EOF
  cat >"$dir/crates/multi/Cargo.toml" <<EOF
[package]
name = "multi"
version.workspace = true

[dependencies.core]
path = "../core"
version = "$pin"
EOF
}

passed=0
failed=0

check() { # name expected_rc version tree
  local name="$1" expected="$2" version="$3" tree="$4"
  local rc=0
  bash "$script" "$version" "$tree" >/dev/null 2>&1 || rc=$?
  if { [ "$expected" = "0" ] && [ "$rc" -eq 0 ]; } ||
    { [ "$expected" = "1" ] && [ "$rc" -ne 0 ]; }; then
    echo "ok   - $name"
    passed=$((passed + 1))
  else
    echo "FAIL - $name (expected rc class $expected, got $rc)"
    failed=$((failed + 1))
  fi
}

# Happy paths.
good="$tmp/good"
make_tree "$good" 1.2.3 "=1.2.3" "1.2"
check "passes on a fully consistent tree" 0 1.2.3 "$good"

rc_tree="$tmp/rc"
make_tree "$rc_tree" 2.0.0-rc.1 "=2.0.0-rc.1" "2.0.0-rc.1"
check "passes a pre-release with exact pins and requirement" 0 2.0.0-rc.1 "$rc_tree"

floatmajor="$tmp/floatmajor"
make_tree "$floatmajor" 1.2.3 "=1.2.3" "1"
check "passes a one-component floating requirement" 0 1.2.3 "$floatmajor"

# Input validation.
check "fails on a v-prefixed version" 1 v1.2.3 "$good"
check "fails on a two-component version" 1 1.2 "$good"

# Whole-tree disagreement: target differs from everything.
check "fails when the target does not match the tree" 1 1.2.4 "$good"

# Localized mismatches.
stale_inline="$tmp/stale-inline"
make_tree "$stale_inline" 1.2.3 "=1.2.3" "1.2"
sed -i 's/version = "=1.2.3"/version = "=0.9.0"/' "$stale_inline/crates/inline/Cargo.toml"
check "fails on a stale inline-table pin" 1 1.2.3 "$stale_inline"

stale_multi="$tmp/stale-multi"
make_tree "$stale_multi" 1.2.3 "=1.2.3" "1.2"
sed -i 's/^version = "=1.2.3"/version = "=0.9.0"/' "$stale_multi/crates/multi/Cargo.toml"
check "fails on a stale multiline-table pin" 1 1.2.3 "$stale_multi"

stale_dev="$tmp/stale-dev"
make_tree "$stale_dev" 1.2.3 "=1.2.3" "1.2"
cat >>"$stale_dev/crates/inline/Cargo.toml" <<'EOF'

[dev-dependencies.core]
path = "../core"
version = "=0.9.0"
EOF
check "fails on a stale dev-dependencies table pin" 1 1.2.3 "$stale_dev"

stale_build="$tmp/stale-build"
make_tree "$stale_build" 1.2.3 "=1.2.3" "1.2"
cat >>"$stale_build/crates/inline/Cargo.toml" <<'EOF'

[build-dependencies.core]
path = "../core"
version = "=0.9.0"
EOF
check "fails on a stale build-dependencies table pin" 1 1.2.3 "$stale_build"

stale_target="$tmp/stale-target"
make_tree "$stale_target" 1.2.3 "=1.2.3" "1.2"
cat >>"$stale_target/crates/inline/Cargo.toml" <<'EOF'

[target.'cfg(unix)'.dependencies.core]
path = "../core"
version = "=0.9.0"
EOF
check "fails on a stale target-specific table pin" 1 1.2.3 "$stale_target"

bad_float="$tmp/bad-float"
make_tree "$bad_float" 1.2.3 "=1.2.3" "1.3"
check "fails when the floating requirement is not satisfied" 1 1.2.3 "$bad_float"

opted_out="$tmp/opted-out"
make_tree "$opted_out" 1.2.3 "=1.2.3" "1.2"
sed -i '/version.workspace = true/d' "$opted_out/crates/core/Cargo.toml"
check "fails when a member has no version at all" 1 1.2.3 "$opted_out"

literal_off="$tmp/literal-off"
make_tree "$literal_off" 1.2.3 "=1.2.3" "1.2"
sed -i 's/version.workspace = true/version = "0.5.0"/' "$literal_off/crates/core/Cargo.toml"
check "fails when a member carries a different literal version" 1 1.2.3 "$literal_off"

literal_on="$tmp/literal-on"
make_tree "$literal_on" 1.2.3 "=1.2.3" "1.2"
sed -i 's/version.workspace = true/version = "1.2.3"/' "$literal_on/crates/core/Cargo.toml"
check "passes when a member carries the matching literal version" 0 1.2.3 "$literal_on"

missing_member="$tmp/missing-member"
make_tree "$missing_member" 1.2.3 "=1.2.3" "1.2"
rm "$missing_member/crates/multi/Cargo.toml"
check "fails when a listed member manifest is missing" 1 1.2.3 "$missing_member"

# Collect-all contract: a tree wrong everywhere reports every finding at once.
all_wrong="$tmp/all-wrong"
make_tree "$all_wrong" 0.0.1 "=0.0.2" "0.3"
out="$(bash "$script" 1.2.3 "$all_wrong" 2>&1 || true)"
finding_count="$(printf '%s\n' "$out" | grep -c '^  - ')"
if [ "$finding_count" -ge 4 ] && printf '%s' "$out" | grep -q 'mismatch(es)'; then
  echo "ok   - reports every mismatch in one run ($finding_count findings)"
  passed=$((passed + 1))
else
  echo "FAIL - reports every mismatch in one run (got $finding_count findings)"
  printf '%s\n' "$out"
  failed=$((failed + 1))
fi

# The real workspace must agree with its own current version.
real_version="$("$root/scripts/ws-version.sh" "$root/Cargo.toml")"
check "passes on this repository at its own version" 0 "$real_version" "$root"

echo
echo "passed=$passed failed=$failed"
[ "$failed" -eq 0 ]
