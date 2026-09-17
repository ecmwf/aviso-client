#!/usr/bin/env bash

# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

#
# Unit tests for package-ffi.sh.
#
# `cargo` is faked with a shim whose cinstall lays out a staging tree the way
# cargo-c does: the real repo headers, a freshly compiled stub shared library
# exporting just enough of the C ABI for the consumer smoke, and a pkg-config
# file carrying absolute staging paths. Everything after the build runs for
# real: the .pc rewrite, the dependency audit, the tarball, the extraction,
# and the compile-and-run of tests/ffi-install/smoke.cpp against the
# extracted copy. Needs a C/C++ compiler and pkg-config; Linux-shaped (the
# macOS branch is exercised on the macOS release runner instead).

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$root/scripts/package-ffi.sh"
for tool in cc c++ pkg-config; do
  command -v "$tool" >/dev/null 2>&1 || {
    echo "these tests need $tool on PATH" >&2
    exit 1
  }
done

tmp="$(mktemp -d "${TMPDIR:-/tmp}/package-ffi-test.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

ws_version="$("$root/scripts/ws-version.sh" "$root/Cargo.toml")"

# The stub implements the symbols exercised by the consumer smoke. Builder
# creation succeeds, while both notify paths reject the malformed identifier
# with the same structured error shape as the real library.
cat >"$tmp/stub.c" <<EOF
#include "$root/crates/aviso-ffi/include/aviso.h"

struct AvisoClient {
  int unused;
};

struct AvisoClientBuilder {
  int unused;
};

struct AvisoOutcome {
  bool ok;
  AvisoClient *client;
  char *text;
  AvisoError error;
};

static AvisoOutcome *error_outcome(void) {
  AvisoOutcome *outcome = calloc(1, sizeof(*outcome));
  if (outcome != NULL) {
    outcome->error.kind = AvisoErrorKind_InvalidInput;
    outcome->error.message = "identifier must be a JSON object";
  }
  return outcome;
}

const char *aviso_version(void) { return "$ws_version"; }

AvisoClientBuilder *aviso_client_builder_new(const char *base_url) {
  (void)base_url;
  return calloc(1, sizeof(AvisoClientBuilder));
}

AvisoOutcome *aviso_client_builder_build(AvisoClientBuilder **builder) {
  AvisoOutcome *outcome = calloc(1, sizeof(*outcome));
  if (outcome == NULL) {
    return NULL;
  }
  free(*builder);
  *builder = NULL;
  outcome->client = calloc(1, sizeof(AvisoClient));
  outcome->ok = outcome->client != NULL;
  if (!outcome->ok) {
    outcome->error.kind = AvisoErrorKind_Internal;
    outcome->error.message = "allocation failed";
  }
  return outcome;
}

void aviso_client_builder_free(AvisoClientBuilder *builder) { free(builder); }
void aviso_client_free(AvisoClient *client) { free(client); }

AvisoOutcome *aviso_client_notify(const AvisoClient *client,
                                  const char *event_type,
                                  const char *identifier_json,
                                  const char *payload_json) {
  (void)client;
  (void)event_type;
  (void)identifier_json;
  (void)payload_json;
  return error_outcome();
}

void aviso_client_notify_async(const AvisoClient *client,
                               const char *event_type,
                               const char *identifier_json,
                               const char *payload_json,
                               void (*on_complete)(void *, AvisoOutcome *),
                               void *ctx) {
  (void)client;
  (void)event_type;
  (void)identifier_json;
  (void)payload_json;
  if (on_complete != NULL) {
    on_complete(ctx, error_outcome());
  }
}

bool aviso_outcome_is_ok(const AvisoOutcome *outcome) {
  return outcome != NULL && outcome->ok;
}

const AvisoError *aviso_outcome_error(const AvisoOutcome *outcome) {
  return outcome == NULL || outcome->ok ? NULL : &outcome->error;
}

char *aviso_outcome_take_string(AvisoOutcome *outcome) {
  if (outcome == NULL) {
    return NULL;
  }
  char *text = outcome->text;
  outcome->text = NULL;
  return text;
}

AvisoClient *aviso_outcome_take_client(AvisoOutcome *outcome) {
  if (outcome == NULL) {
    return NULL;
  }
  AvisoClient *client = outcome->client;
  outcome->client = NULL;
  return client;
}

void aviso_outcome_free(AvisoOutcome *outcome) {
  if (outcome != NULL) {
    free(outcome->client);
    free(outcome->text);
    free(outcome);
  }
}

void aviso_string_free(char *text) { free(text); }
EOF

# The audit fixture: a foreign shared library whose symbol the stub variant
# really calls, so the NEEDED entry survives the linker's --as-needed and
# must trip the dependency allowlist.
cat >"$tmp/fake.c" <<'EOF'
int fake_dep(void) { return 1; }
EOF
cc -shared -fPIC -o "$tmp/libcfake.so" "$tmp/fake.c"
cat >"$tmp/stub_audit.c" <<EOF
extern int fake_dep(void);
const char *aviso_version(void) { return fake_dep() ? "$ws_version" : ""; }
EOF

# Fake cargo: `cinstall ... --prefix P --libdir L` builds the staging tree.
mkdir -p "$tmp/bin"
cat >"$tmp/bin/cargo" <<EOF
#!/usr/bin/env bash
set -euo pipefail
prefix="" libdir="" prev=""
for a in "\$@"; do
  [ "\$prev" = "--prefix" ] && prefix="\$a"
  [ "\$prev" = "--libdir" ] && libdir="\$a"
  prev="\$a"
done
[ -n "\$prefix" ] && [ -n "\$libdir" ]
mkdir -p "\$prefix/include/aviso_ffi" "\$libdir/pkgconfig"
cp "$root/crates/aviso-ffi/include/aviso.h" \
   "$root/crates/aviso-ffi/include/aviso.hpp" "\$prefix/include/aviso_ffi/"
cc -shared -fPIC -o "\$libdir/libaviso_ffi.so" "\${STUB_SRC:-$tmp/stub.c}" \${STUB_EXTRA_LIBS:-}
: >"\$libdir/libaviso_ffi.a"
cat >"\$libdir/pkgconfig/aviso_ffi.pc" <<PC
prefix=\$prefix
exec_prefix=\$prefix
libdir=\$libdir
includedir=\$prefix/include
Name: aviso_ffi
Description: stub
Version: $ws_version
Libs: -L\\\${libdir} -laviso_ffi
Cflags: -I\\\${includedir}/aviso_ffi
PC
EOF
chmod +x "$tmp/bin/cargo"

passed=0
failed=0

# Happy path: build, relocate, pack, and verify from the extracted tarball.
out="$tmp/out"
rc=0
PATH="$tmp/bin:$PATH" bash "$script" "$ws_version" linux-test "$out" \
  >"$tmp/run.log" 2>&1 || rc=$?
if [ "$rc" -eq 0 ] && [ -f "$out/aviso-ffi-$ws_version-linux-test.tar.gz" ]; then
  echo "ok   - builds and verifies a relocatable tarball"
  passed=$((passed + 1))
else
  echo "FAIL - builds and verifies a relocatable tarball (rc=$rc)"
  cat "$tmp/run.log"
  failed=$((failed + 1))
fi

# The packed .pc must be pcfiledir-relative with no staging path left. The
# extraction is guarded so a missing tarball records a failure instead of
# aborting the remaining cases under set -e.
pc="$tmp/aviso-ffi-$ws_version-linux-test/lib/pkgconfig/aviso_ffi.pc"
if [ -f "$out/aviso-ffi-$ws_version-linux-test.tar.gz" ]; then
  tar -xzf "$out/aviso-ffi-$ws_version-linux-test.tar.gz" -C "$tmp"
fi
if [ -f "$pc" ] && grep -q '^prefix=[$]{pcfiledir}/../..$' "$pc" && ! grep -qF "$tmp" "$pc"; then
  echo "ok   - the shipped .pc is relocatable"
  passed=$((passed + 1))
else
  echo "FAIL - the shipped .pc is relocatable"
  [ -f "$pc" ] && cat "$pc"
  failed=$((failed + 1))
fi

# Legal files must survive staging and extraction without modification.
for legal_file in LICENSE NOTICE; do
  if cmp -s "$root/$legal_file" "$tmp/aviso-ffi-$ws_version-linux-test/$legal_file"; then
    echo "ok   - the tarball preserves $legal_file"
    passed=$((passed + 1))
  else
    echo "FAIL - the tarball preserves $legal_file"
    failed=$((failed + 1))
  fi
done

# A version that does not match the workspace must be refused before building.
rc=0
PATH="$tmp/bin:$PATH" bash "$script" 9.9.9 linux-test "$tmp/out2" >/dev/null 2>&1 || rc=$?
if [ "$rc" -ne 0 ]; then
  echo "ok   - refuses a version that is not the workspace version"
  passed=$((passed + 1))
else
  echo "FAIL - refuses a version that is not the workspace version"
  failed=$((failed + 1))
fi

# A library linking beyond the system allowlist must fail the audit.
rc=0
STUB_SRC="$tmp/stub_audit.c" STUB_EXTRA_LIBS="-L$tmp -lcfake" PATH="$tmp/bin:$PATH" \
  bash "$script" "$ws_version" linux-audit "$tmp/out3" >"$tmp/audit.log" 2>&1 || rc=$?
if [ "$rc" -ne 0 ] && grep -q "unexpected shared-library dependencies" "$tmp/audit.log"; then
  echo "ok   - the dependency audit rejects an unexpected library"
  passed=$((passed + 1))
else
  echo "FAIL - the dependency audit rejects an unexpected library (rc=$rc)"
  cat "$tmp/audit.log"
  failed=$((failed + 1))
fi

echo
echo "passed=$passed failed=$failed"
[ "$failed" -eq 0 ]
