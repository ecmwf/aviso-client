#!/usr/bin/env bash
#
# Unit tests for publish-crates.sh.
#
# `curl` and `cargo` are faked: the curl shim serves a directory of sparse-
# index fixtures (recording every requested path) and the cargo shim records
# publish order, "indexes" published crates by writing the fixture entry, and
# packages deterministic .crate files. The guard, the retry checksum logic,
# the yanked refusal, the poll timeout, and the fail-closed probe are all
# exercised offline.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$root/scripts/publish-crates.sh"
command -v jq >/dev/null 2>&1 || {
  echo "these tests need jq on PATH" >&2
  exit 1
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Shared with the shims: where index fixtures live, and the request/call logs.
export INDEX_FIXTURE="" CURL_LOG="" CALL_LOG="" TEST_VERSION="9.9.9"

mkdir -p "$tmp/bin"

# Fake curl: serve $INDEX_FIXTURE/<path> for the requested URL, print the
# HTTP code (200, 404, or the contents of <path>.code when present), and log
# the path so tests can assert the index layout.
cat >"$tmp/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
out="" url="" prev=""
for a in "$@"; do
  [ "$prev" = "-o" ] && out="$a"
  case "$a" in
  http://*|https://*) url="$a" ;;
  esac
  prev="$a"
done
path="${url#*//index.test/}"
echo "$path" >>"$CURL_LOG"
if [ -f "$INDEX_FIXTURE/$path.code" ]; then
  cat "$INDEX_FIXTURE/$path.code"
elif [ -f "$INDEX_FIXTURE/$path" ]; then
  cp "$INDEX_FIXTURE/$path" "$out"
  printf 200
else
  printf 404
fi
EOF
chmod +x "$tmp/bin/curl"

# Fake cargo: `publish -p <crate>` logs the call and (unless PUBLISH_SILENT)
# writes the crate's index entry with the checksum of the deterministic
# package payload; `package -p <crate>` writes that payload under
# target/package/.
cat >"$tmp/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
sub="$1"
crate="" prev=""
for a in "$@"; do
  [ "$prev" = "-p" ] && crate="$a"
  prev="$a"
done
index_path() {
  local name="$1"
  case "${#name}" in
  1) printf '1/%s' "$name" ;;
  2) printf '2/%s' "$name" ;;
  3) printf '3/%s/%s' "${name:0:1}" "$name" ;;
  *) printf '%s/%s/%s' "${name:0:2}" "${name:2:2}" "$name" ;;
  esac
}
payload() { printf 'crate-payload %s %s' "$1" "$TEST_VERSION"; }
case "$sub" in
publish)
  echo "publish $crate" >>"$CALL_LOG"
  if [ "${PUBLISH_SILENT:-}" != "true" ]; then
    p="$INDEX_FIXTURE/$(index_path "$crate")"
    mkdir -p "$(dirname "$p")"
    cksum="$(payload "$crate" | sha256sum | cut -d' ' -f1)"
    printf '{"name":"%s","vers":"%s","cksum":"%s","yanked":false}\n' \
      "$crate" "$TEST_VERSION" "$cksum" >>"$p"
  fi
  ;;
package)
  mkdir -p target/package
  payload "$crate" >"target/package/$crate-$TEST_VERSION.crate"
  ;;
*)
  echo "fake cargo: unexpected subcommand $sub" >&2
  exit 2
  ;;
esac
EOF
chmod +x "$tmp/bin/cargo"

# A minimal workspace manifest so the script's version cross-check passes.
cat >"$tmp/Cargo.toml" <<EOF
[workspace.package]
version = "$TEST_VERSION"
EOF

passed=0
failed=0

run_case() { # name expected_rc retry_mode silent fixture_setup_fn [crates...]
  local name="$1" expected="$2" retry="$3" silent="$4" setup="$5"
  shift 5
  local fixture rc=0
  fixture="$(mktemp -d "$tmp/fix-XXXXXX")"
  INDEX_FIXTURE="$fixture" "$setup" "$fixture"
  local workdir
  workdir="$(mktemp -d "$tmp/work-XXXXXX")"
  CURL_LOG="$(mktemp "$tmp/curl-XXXXXX.log")"
  CALL_LOG="$(mktemp "$tmp/call-XXXXXX.log")"
  (
    cd "$workdir"
    PATH="$tmp/bin:$PATH" INDEX_FIXTURE="$fixture" \
      CURL_LOG="$CURL_LOG" CALL_LOG="$CALL_LOG" \
      RETRY_MODE="$retry" PUBLISH_SILENT="$silent" \
      INDEX_URL="https://index.test" MANIFEST="$tmp/Cargo.toml" \
      POLL_SECONDS=2 POLL_INTERVAL=1 \
      bash "$script" "$TEST_VERSION" "$@"
  ) >/dev/null 2>&1 || rc=$?
  if { [ "$expected" = "0" ] && [ "$rc" -eq 0 ]; } ||
    { [ "$expected" = "1" ] && [ "$rc" -ne 0 ]; }; then
    echo "ok   - $name"
    passed=$((passed + 1))
  else
    echo "FAIL - $name (expected rc class $expected, got $rc)"
    failed=$((failed + 1))
  fi
}

no_setup() { :; }

seed_aviso() { # fixture: aviso already indexed at the target version
  mkdir -p "$1/av/is"
  printf '{"name":"aviso","vers":"%s","cksum":"%s","yanked":false}\n' \
    "$TEST_VERSION" "0000000000000000000000000000000000000000000000000000000000000000" \
    >"$1/av/is/aviso"
}

seed_finesse_matching() { # fixture: finesse indexed with this checkout's checksum
  mkdir -p "$1/fi/ne"
  local cksum
  cksum="$(printf 'crate-payload %s %s' finesse "$TEST_VERSION" | sha256sum | cut -d' ' -f1)"
  printf '{"name":"finesse","vers":"%s","cksum":"%s","yanked":false}\n' \
    "$TEST_VERSION" "$cksum" >"$1/fi/ne/finesse"
}

seed_finesse_mismatch() { # fixture: finesse indexed with a foreign checksum
  mkdir -p "$1/fi/ne"
  printf '{"name":"finesse","vers":"%s","cksum":"%s","yanked":false}\n' \
    "$TEST_VERSION" "1111111111111111111111111111111111111111111111111111111111111111" \
    >"$1/fi/ne/finesse"
}

seed_late_mismatch() { # fixture: finesse absent, but aviso indexed divergent
  mkdir -p "$1/av/is"
  printf '{"name":"aviso","vers":"%s","cksum":"%s","yanked":false}\n' \
    "$TEST_VERSION" "1111111111111111111111111111111111111111111111111111111111111111" \
    >"$1/av/is/aviso"
}

seed_finesse_yanked() { # fixture: finesse yanked, recorded as a later update
  # Written as an original non-yanked line followed by a yanked one for the
  # same version, so a reader taking the FIRST match instead of the latest
  # would wrongly see a live entry and regress this case.
  mkdir -p "$1/fi/ne"
  local cksum
  cksum="$(printf 'crate-payload %s %s' finesse "$TEST_VERSION" | sha256sum | cut -d' ' -f1)"
  {
    printf '{"name":"finesse","vers":"%s","cksum":"%s","yanked":false}\n' \
      "$TEST_VERSION" "$cksum"
    printf '{"name":"finesse","vers":"%s","cksum":"%s","yanked":true}\n' \
      "$TEST_VERSION" "$cksum"
  } >"$1/fi/ne/finesse"
}

seed_server_error() { # fixture: the finesse probe answers HTTP 500
  mkdir -p "$1/fi/ne"
  printf 500 >"$1/fi/ne/finesse.code"
}

seed_garbage_body() { # fixture: 200 with a non-JSON body (a proxy error page)
  mkdir -p "$1/fi/ne"
  printf '<html>bad gateway</html>\n' >"$1/fi/ne/finesse"
}

seed_empty_body() { # fixture: 200 with an empty body
  mkdir -p "$1/fi/ne"
  : >"$1/fi/ne/finesse"
}

seed_old_version() { # fixture: finesse indexed at a DIFFERENT version only
  mkdir -p "$1/fi/ne"
  printf '{"name":"finesse","vers":"1.0.0","cksum":"%s","yanked":false}\n' \
    "2222222222222222222222222222222222222222222222222222222222222222" \
    >"$1/fi/ne/finesse"
}

# Fresh publish: nothing indexed, everything publishes and propagates.
run_case "publishes a fresh release end to end" 0 false false no_setup

# An older version on the index is normal and must not trip the guard.
run_case "ignores other versions already on the index" 0 false false seed_old_version

# Release-start guard.
run_case "fails loud when the target version pre-exists" 1 false false seed_aviso
last_call_log="$CALL_LOG"
if [ ! -s "$last_call_log" ]; then
  echo "ok   - the guard publishes nothing"
  passed=$((passed + 1))
else
  echo "FAIL - the guard publishes nothing (calls: $(paste -sd, "$last_call_log"))"
  failed=$((failed + 1))
fi

# Retry semantics.
run_case "retry skips an indexed crate with a matching checksum" 0 true false seed_finesse_matching
last_call_log="$CALL_LOG"
if [ "$(paste -sd' ' "$last_call_log")" = "publish aviso publish aviso-cli publish aviso-ffi" ]; then
  echo "ok   - retry publishes only the remaining crates, in order"
  passed=$((passed + 1))
else
  echo "FAIL - retry publishes only the remaining crates (calls: $(paste -sd' ' "$last_call_log"))"
  failed=$((failed + 1))
fi
run_case "retry fails on a checksum mismatch" 1 true false seed_finesse_mismatch
run_case "retry fails on a yanked target version" 1 true false seed_finesse_yanked

# The mismatch must surface BEFORE anything is published: an absent earlier
# crate must not be published when a later crate's indexed artifact diverges.
run_case "retry verifies every indexed crate before publishing" 1 true false seed_late_mismatch
last_call_log="$CALL_LOG"
if ! grep -q '^publish ' "$last_call_log"; then
  echo "ok   - a late mismatch publishes nothing"
  passed=$((passed + 1))
else
  echo "FAIL - a late mismatch publishes nothing (calls: $(paste -sd' ' "$last_call_log"))"
  failed=$((failed + 1))
fi

# Failure modes.
run_case "fails when the index never shows the publish" 1 false true no_setup
run_case "fails closed on a non-404 index error" 1 false false seed_server_error
run_case "fails closed on a 200 with a non-JSON body" 1 false false seed_garbage_body
run_case "fails closed on a 200 with an empty body" 1 false false seed_empty_body

# Version cross-check: the argument must match the workspace version.
rc=0
(
  cd "$tmp"
  PATH="$tmp/bin:$PATH" INDEX_FIXTURE="$tmp/none" CURL_LOG="$tmp/c.log" \
    CALL_LOG="$tmp/k.log" MANIFEST="$tmp/Cargo.toml" INDEX_URL="https://index.test" \
    bash "$script" 1.0.0
) >/dev/null 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then
  echo "ok   - rejects a version that is not the workspace version"
  passed=$((passed + 1))
else
  echo "FAIL - rejects a version that is not the workspace version"
  failed=$((failed + 1))
fi

# Index layout: every name-length bucket resolves to the documented path.
run_case "handles every index path shape" 0 false false no_setup a ab abc abcd
last_curl_log="$CURL_LOG"
for p in 1/a 2/ab 3/a/abc ab/cd/abcd; do
  if grep -qx "$p" "$last_curl_log"; then
    echo "ok   - probes index path $p"
    passed=$((passed + 1))
  else
    echo "FAIL - probes index path $p (saw: $(sort -u "$last_curl_log" | paste -sd' '))"
    failed=$((failed + 1))
  fi
done

echo
echo "passed=$passed failed=$failed"
[ "$failed" -eq 0 ]
