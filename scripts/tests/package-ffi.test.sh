#!/usr/bin/env bash
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

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

ws_version="$("$root/scripts/ws-version.sh" "$root/Cargo.toml")"

# The stub implements the one symbol the consumer smoke links,
# aviso_version, returning the live workspace version.
cat >"$tmp/stub.c" <<EOF
const char *aviso_version(void) { return "$ws_version"; }
EOF

# The audit fixture: a foreign shared library whose symbol the stub variant
# really calls, so the NEEDED entry survives the linker's --as-needed and
# must trip the dependency allowlist.
cat >"$tmp/fake.c" <<'EOF'
int fake_dep(void) { return 1; }
EOF
cc -shared -fPIC -o "$tmp/libfake.so" "$tmp/fake.c"
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
if [ -f "$pc" ] && grep -q '^prefix=[$]{pcfiledir}/../..$' "$pc" && ! grep -q "$tmp" "$pc"; then
  echo "ok   - the shipped .pc is relocatable"
  passed=$((passed + 1))
else
  echo "FAIL - the shipped .pc is relocatable"
  [ -f "$pc" ] && cat "$pc"
  failed=$((failed + 1))
fi

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
STUB_SRC="$tmp/stub_audit.c" STUB_EXTRA_LIBS="-L$tmp -lfake" PATH="$tmp/bin:$PATH" \
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
