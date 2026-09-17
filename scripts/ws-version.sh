#!/usr/bin/env bash

# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

#
# Print the [workspace.package] version from the workspace Cargo.toml.
#
# Section-scoped so it never picks up a `version` key in another table and does
# not depend on file ordering. This is the single reader of the release
# version: both the justfile and the release-invariant check call it, so the
# manifest stays the one source of truth.
#
# Usage: ws-version.sh [path/to/Cargo.toml]   (defaults to ./Cargo.toml)

set -euo pipefail

manifest="${1:-Cargo.toml}"
[ -f "$manifest" ] || {
  echo "ws-version: manifest '$manifest' not found" >&2
  exit 1
}

version="$(awk '
  /^\[/ { section = $0 }
  section == "[workspace.package]" && /^version[[:space:]]*=/ {
    if (match($0, /"[^"]+"/)) {
      print substr($0, RSTART + 1, RLENGTH - 2)
      exit
    }
  }
' "$manifest")"

[ -n "$version" ] || {
  echo "ws-version: no [workspace.package] version in '$manifest'" >&2
  exit 1
}

printf '%s\n' "$version"
