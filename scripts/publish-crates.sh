#!/usr/bin/env bash

# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

#
# Ordered, fail-loud, retry-safe crates.io publish.
#
# Publishes the crates in dependency order, waiting for each one to appear on
# the sparse index before the next (a dependent publish would otherwise race
# index propagation). crates.io versions are immutable, so the script is
# strict about what it finds on the index BEFORE touching anything:
#
#   - Release start (the default): the target version must not exist on the
#     index for any crate. A pre-existing version is an error, never a no-op;
#     every hit is reported and nothing is published.
#   - Retry (RETRY_MODE=true, after a partial failure): a crate already
#     indexed at the target version is skipped only when the `.crate` packaged
#     from THIS checkout has the same sha256 as the indexed one and the entry
#     is not yanked. A checksum mismatch or a yanked entry stops the run: the
#     indexed artifact is not what this tag builds, and a yanked version can
#     never be re-published, so the release moves to a new version instead.
#
# A GitHub re-run after a partial publish hits the release-start guard by
# design; the recovery path is a manual dispatch on the tag with retry
# enabled, exactly because that path re-verifies checksums first.
#
# Usage: publish-crates.sh <version> [crate ...]
#   crates default to the publish-ordered set; the version must equal the
#   workspace version (cross-checked, so a stray direct invocation cannot
#   publish a mismatched tree).
#
# Environment:
#   CARGO_REGISTRY_TOKEN  consumed by cargo publish itself
#   RETRY_MODE            "true" enables the checksum-verified skip (default false)
#   INDEX_URL             sparse index base (default https://index.crates.io)
#   POLL_SECONDS          per-crate index-propagation deadline (default 60)
#   POLL_INTERVAL         seconds between polls (default 5)
#   MANIFEST              workspace Cargo.toml (default: the repo root's)

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

version="${1:-}"
shift || true
crates=("$@")
[ "${#crates[@]}" -gt 0 ] || crates=(finesse aviso aviso-cli aviso-ffi)

RETRY_MODE="${RETRY_MODE:-false}"
INDEX_URL="${INDEX_URL:-https://index.crates.io}"
POLL_SECONDS="${POLL_SECONDS:-60}"
POLL_INTERVAL="${POLL_INTERVAL:-5}"
MANIFEST="${MANIFEST:-$here/../Cargo.toml}"

die() {
  echo "publish-crates: $*" >&2
  exit 1
}

[ -n "$version" ] || die "usage: publish-crates.sh <version> [crate ...]"
command -v jq >/dev/null 2>&1 || die "jq is required"

ws_version="$("$here/ws-version.sh" "$MANIFEST")"
[ "$version" = "$ws_version" ] ||
  die "version '$version' does not match the workspace version '$ws_version'"

# Anchor every cargo invocation (and the target/package/ paths they produce)
# to the workspace that was just version-checked, wherever the script was
# invoked from.
cd "$(dirname "$MANIFEST")"

# Sparse-index path for a crate name, per the registry layout:
# 1-3 character names live under length buckets, longer names under the
# first-two/next-two character prefix. e.g. aviso -> av/is/aviso.
index_path() {
  local name="$1"
  case "${#name}" in
  1) printf '1/%s' "$name" ;;
  2) printf '2/%s' "$name" ;;
  3) printf '3/%s/%s' "${name:0:1}" "$name" ;;
  *) printf '%s/%s/%s' "${name:0:2}" "${name:2:2}" "$name" ;;
  esac
}

# Probe the index for crate@version. Sets probe_state to 'indexed' or
# 'absent', and on 'indexed' also probe_cksum and probe_yanked. A 404 means
# the name has never been published (absent); a 200 whose body is empty or
# not the JSON-lines format, and any other answer, fail closed so an index
# or proxy anomaly is never read as "safe to publish".
probe_body="$(mktemp)"
trap 'rm -f "$probe_body"' EXIT
probe_entry() {
  local crate="$1" code
  code="$(curl -sS -o "$probe_body" -w '%{http_code}' "$INDEX_URL/$(index_path "$crate")")" ||
    die "could not reach the index for '$crate'"
  case "$code" in
  200)
    [ -s "$probe_body" ] ||
      die "the index answered 200 for '$crate' with an empty body; failing closed"
    # Every line must be an index entry (an object carrying at least string
    # vers and cksum); a JSON error page like {"error":"bad gateway"} must
    # not read as "version absent".
    jq -se 'all(.[]; type == "object" and (.vers | type == "string") and (.cksum | type == "string"))' \
      "$probe_body" >/dev/null 2>&1 ||
      die "the index answered 200 for '$crate' with a body that is not the sparse-index JSON-lines format; failing closed"
    local entry
    # One line per version is the index contract (yanks rewrite the line in
    # place), but read the LAST match defensively so a hypothetical update
    # appended later can never be shadowed by a stale entry.
    entry="$(jq -c --arg v "$version" 'select(.vers == $v)' "$probe_body" | tail -n 1)"
    if [ -n "$entry" ]; then
      probe_state=indexed
      probe_cksum="$(jq -r '.cksum' <<<"$entry")"
      probe_yanked="$(jq -r '.yanked // false' <<<"$entry")"
    else
      probe_state=absent
    fi
    ;;
  404)
    probe_state=absent
    ;;
  *)
    die "index probe for '$crate' returned HTTP $code; failing closed"
    ;;
  esac
}

# --- release-start guard ------------------------------------------------------
# Outside retry mode, any crate already carrying the target version means this
# is not a fresh release start: report every hit at once and stop.
indexed_hits=()
for crate in "${crates[@]}"; do
  probe_entry "$crate"
  [ "$probe_state" = "indexed" ] && indexed_hits+=("$crate")
done
if [ "${#indexed_hits[@]}" -gt 0 ] && [ "$RETRY_MODE" != "true" ]; then
  echo "publish-crates: $version already exists on the index for: ${indexed_hits[*]}" >&2
  echo "  A pre-existing version at release start is an error, not a no-op." >&2
  echo "  If this is a resume after a partial failure, dispatch the publish" >&2
  echo "  with the retry input enabled so each skip is checksum-verified." >&2
  exit 1
fi

# --- retry preflight ----------------------------------------------------------
# Every already-indexed crate is verified BEFORE anything is published. If the
# verification ran lazily inside the publish loop, a mismatch or yank on a
# later crate would only surface after earlier crates were published, leaving
# extra immutable versions behind in a run that then declares itself unsafe.
verify_indexed() {
  local crate="$1" local_cksum
  [ "$probe_yanked" = "false" ] ||
    die "'$crate' $version is on the index but yanked; a yanked version cannot be re-published. Release a new version instead"
  echo "--- $crate $version is already indexed; verifying it is this tag's artifact"
  cargo package --locked -p "$crate" >/dev/null
  local_cksum="$(sha256sum "target/package/$crate-$version.crate" | cut -d' ' -f1)"
  [ "$local_cksum" = "$probe_cksum" ] ||
    die "'$crate' $version on the index has checksum $probe_cksum, but this checkout packages to $local_cksum; the indexed artifact is not this tag's"
  echo "    checksum matches; will skip"
}

declare -A verified_skip=()
if [ "$RETRY_MODE" = "true" ]; then
  for crate in "${crates[@]}"; do
    probe_entry "$crate"
    [ "$probe_state" = "indexed" ] || continue
    verify_indexed "$crate"
    verified_skip["$crate"]=1
  done
fi

# --- ordered publish with index polling --------------------------------------
for crate in "${crates[@]}"; do
  if [ -n "${verified_skip[$crate]:-}" ]; then
    echo "--- $crate $version verified during the retry preflight; skipping"
    continue
  fi
  probe_entry "$crate"
  [ "$probe_state" = "absent" ] ||
    die "'$crate' became indexed at $version mid-run; stopping"

  echo "--- cargo publish -p $crate ($version)"
  cargo publish --locked -p "$crate"

  echo "    waiting for $crate $version to appear on the index"
  deadline=$((SECONDS + POLL_SECONDS))
  until probe_entry "$crate" && [ "$probe_state" = "indexed" ]; do
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "publish-crates: '$crate' $version did not appear on the index within ${POLL_SECONDS}s." >&2
      echo "  The upload may still be propagating. Once the cause is clear," >&2
      echo "  resume with a dispatch on the tag with retry enabled." >&2
      exit 1
    fi
    sleep "$POLL_INTERVAL"
  done
  echo "    indexed"
done

echo "publish-crates: all of '${crates[*]}' published at $version"
