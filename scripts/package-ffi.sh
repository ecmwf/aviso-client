#!/usr/bin/env bash

# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

#
# Build, relocate, pack, and verify one prebuilt aviso-ffi tarball.
#
# `cargo cinstall` writes the staging prefix into its outputs: the pkg-config
# file's prefix/libdir/includedir, and on macOS the dylib install name. A
# tarball of the raw staging tree would only work at that exact path, so this
# script rewrites both to relocatable forms, then PROVES relocatability by
# extracting the finished tarball into a fresh directory and building and
# running the pkg-config consumer against the extracted copy, never the
# staging tree. The shipped libraries are also audited against an allowlist
# of system dependencies, so an accidental OpenSSL (or anything else) link
# fails the build instead of the consumer.
#
# Usage: package-ffi.sh <version> <platform-label> [out-dir]
#   version         must equal the workspace version
#   platform-label  tarball suffix, e.g. linux-x86_64, macos-arm64
#   out-dir         where the tarball lands (default dist-ffi)
#
# Environment:
#   TARGET          optional rust target triple for cross builds
#   SMOKE_CXXFLAGS  extra compiler flags for the consumer smoke (e.g.
#                   "-arch x86_64" when cross-building the Intel macOS slice)

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

version="${1:-}"
platform="${2:-}"
out_dir="${3:-dist-ffi}"

die() {
  echo "package-ffi: $*" >&2
  exit 1
}

if [ -z "$version" ] || [ -z "$platform" ]; then
  die "usage: package-ffi.sh <version> <platform-label> [out-dir]"
fi

ws_version="$("$here/ws-version.sh" "$here/../Cargo.toml")"
[ "$version" = "$ws_version" ] ||
  die "version '$version' does not match the workspace version '$ws_version'"

cd "$here/.."
command -v pkg-config >/dev/null 2>&1 || die "pkg-config is required"

name="aviso-ffi-$version-$platform"
work="$(mktemp -d "${TMPDIR:-/tmp}/package-ffi.XXXXXX")"
trap 'rm -rf "$work"' EXIT
staging="$work/pack/$name"
mkdir -p "$staging" "$out_dir"

# --- build into the staging prefix --------------------------------------------
# The ${arr[@]+...} expansions keep empty arrays safe under set -u on the
# bash 3.2 that macOS ships.
target_args=()
[ -n "${TARGET:-}" ] && target_args=(--target "$TARGET")
cargo cinstall --locked -p aviso-ffi --release \
  ${target_args[@]+"${target_args[@]}"} \
  --prefix "$staging" --libdir "$staging/lib"

# --- make the layout relocatable ----------------------------------------------
# The .pc must locate everything relative to its own position, whatever
# directory the consumer unpacked into.
pc="$staging/lib/pkgconfig/aviso_ffi.pc"
[ -f "$pc" ] || die "cinstall produced no $pc"
tmp_pc="$work/aviso_ffi.pc.new"
awk '
  /^prefix=/      { print "prefix=${pcfiledir}/../.."; next }
  /^exec_prefix=/ { print "exec_prefix=${prefix}"; next }
  /^libdir=/      { print "libdir=${exec_prefix}/lib"; next }
  /^includedir=/  { print "includedir=${prefix}/include"; next }
  { print }
' "$pc" >"$tmp_pc"
mv "$tmp_pc" "$pc"
grep -q '^prefix=[$]{pcfiledir}' "$pc" || die "failed to rewrite $pc"

if [ "$(uname)" = "Darwin" ]; then
  dylib="$staging/lib/libaviso_ffi.dylib"
  [ -f "$dylib" ] || die "no dylib in the staging tree"
  install_name_tool -id "@rpath/libaviso_ffi.dylib" "$dylib"
  otool -D "$dylib" | grep -q '@rpath/libaviso_ffi.dylib' ||
    die "dylib install name was not rewritten"
fi

# --- dependency audit ----------------------------------------------------------
# rustls-based, so nothing beyond the base system runtime may appear.
if [ "$(uname)" = "Darwin" ]; then
  deps="$(otool -L "$staging/lib/libaviso_ffi.dylib" | tail -n +2 | awk '{print $1}')"
  bad="$(printf '%s\n' "$deps" | grep -Ev '^(@rpath/libaviso_ffi\.dylib|/usr/lib/|/System/)' || true)"
else
  deps="$(ldd "$staging/lib/libaviso_ffi.so" | awk '{print $1}')"
  # Anchored to full sonames so e.g. libcrypto cannot ride on a libc
  # prefix. The dynamic loader's own line is an absolute path that varies
  # by distro (/lib64/ld-linux-x86-64.so.2, /usr/lib64/...,
  # /lib/ld-linux-aarch64.so.1); a library with no dynamic dependencies at
  # all makes ldd print "statically linked", the safest possible answer.
  bad="$(printf '%s\n' "$deps" |
    grep -Ev '^((linux-vdso|libgcc_s|libm|libc|libpthread|libdl|librt)\.so(\.[0-9]+)*|statically|(/[A-Za-z0-9._/-]*/)?ld-linux[A-Za-z0-9_-]*\.so(\.[0-9]+)*)$' || true)"
fi
[ -z "$bad" ] || die "unexpected shared-library dependencies: $bad"

# --- pack ----------------------------------------------------------------------
tarball="$out_dir/$name.tar.gz"
tar -C "$work/pack" -czf "$tarball" "$name"

# --- prove the tarball relocates -----------------------------------------------
extract="$work/extract"
mkdir -p "$extract"
tar -C "$extract" -xzf "$tarball"
root="$extract/$name"

export PKG_CONFIG_PATH="$root/lib/pkgconfig"
pkg-config --exists aviso_ffi || die "pkg-config cannot find the extracted package"
modversion="$(pkg-config --modversion aviso_ffi)"
[ "$modversion" = "$version" ] ||
  die "extracted package reports version '$modversion', expected '$version'"

smoke_flags=()
[ -n "${SMOKE_CXXFLAGS:-}" ] && read -ra smoke_flags <<<"$SMOKE_CXXFLAGS"
# Word-splitting the pkg-config output is the point here; each flag must be a
# separate compiler argument.
# shellcheck disable=SC2046
"${CXX:-c++}" -std=c++17 ${smoke_flags[@]+"${smoke_flags[@]}"} \
  tests/ffi-install/smoke.cpp \
  $(pkg-config --cflags --libs aviso_ffi) \
  -Wl,-rpath,"$root/lib" -o "$work/smoke"
smoke_out="$(LD_LIBRARY_PATH="$root/lib" DYLD_LIBRARY_PATH="$root/lib" "$work/smoke")"
echo "$smoke_out"
printf '%s' "$smoke_out" | grep -qF "$version" ||
  die "the consumer smoke printed '$smoke_out', expected it to contain '$version'"

echo "package-ffi: built and verified $tarball"
