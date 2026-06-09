#!/usr/bin/env bash
#
# Check that one target version is consistent across every workspace manifest.
#
# The release publishes all crates and the Python distribution under a single
# shared version, so before anything irreversible happens every place that
# names that version must agree:
#
#   1. [workspace.package] version in the root Cargo.toml,
#   2. every workspace member (it inherits via `version.workspace = true` or
#      carries the same literal),
#   3. every workspace-internal dependency that pairs `path` with a `version`
#      requirement: an exact `=x.y.z` pin must equal the target exactly, and a
#      floating requirement (e.g. `finesse = "2.0"`) must be satisfied by it.
#
# pyproject.toml is deliberately not read: its version is `dynamic`, resolved
# by maturin from the workspace, and the built wheel/sdist metadata is checked
# by the preflight instead.
#
# All mismatches are collected and reported together, then the script fails
# once with the count, so one run shows the whole repair list.
#
# Usage: version-consistency.sh <version> [workspace-root]
#   version         target release version, bare semver (e.g. 2.0.0, 2.0.0-rc.1)
#   workspace-root  defaults to the repository root above this script

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
version="${1:-}"
root="${2:-$here/..}"

die() {
  echo "version-consistency: $*" >&2
  exit 1
}

[ -n "$version" ] || die "usage: version-consistency.sh <version> [workspace-root]"
[ -f "$root/Cargo.toml" ] || die "no Cargo.toml under '$root'"

# Bare semver with optional pre-release / build metadata, the same shape the
# release tag must have. Valid: 2.0.0, 2.0.0-rc.1. Invalid: v2.0.0, 2.0.
semver_re='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'
printf '%s' "$version" | grep -Eq "$semver_re" ||
  die "'$version' is not a bare semver version"

mismatches=()

# --- 1. the workspace version ------------------------------------------------
ws_version="$("$here/ws-version.sh" "$root/Cargo.toml")"
[ "$ws_version" = "$version" ] ||
  mismatches+=("Cargo.toml: [workspace.package] version is '$ws_version'")

# --- manifest scanning -------------------------------------------------------
# Emits one line per fact found in a manifest, section-aware so a `version`
# key in an unrelated table is never picked up:
#   pkg|literal|<version>   a [package] table with a literal version
#   pkg|inherit|-           a [package] table inheriting the workspace version
#   dep|<name>|<req>        a dependency pairing `path` with a `version`
#                           requirement (inline table or [*dependencies.<name>]
#                           table); dependencies without `path` are external
#                           and not this script's business.
scan_manifest() {
  awk '
    function flush_dep() {
      if (dep != "" && dep_path && dep_ver != "")
        printf "dep|%s|%s\n", dep, dep_ver
      dep = ""; dep_path = 0; dep_ver = ""
    }
    function quoted(s) {
      if (match(s, /"[^"]*"/))
        return substr(s, RSTART + 1, RLENGTH - 2)
      return ""
    }
    /^[[:space:]]*\[/ {
      flush_dep()
      section = $0
      sub(/^[[:space:]]*/, "", section)
      if (section ~ /^\[([^]]*\.)?(dev-|build-)?dependencies\.[A-Za-z0-9_-]+\]/) {
        dep = section
        sub(/\]$/, "", dep)
        sub(/^.*dependencies\./, "", dep)
      }
      next
    }
    section == "[package]" {
      if ($0 ~ /^version[[:space:]]*=[[:space:]]*"/)
        printf "pkg|literal|%s\n", quoted($0)
      else if ($0 ~ /^version\.workspace[[:space:]]*=[[:space:]]*true/)
        print "pkg|inherit|-"
    }
    dep != "" {
      if ($0 ~ /^path[[:space:]]*=/) dep_path = 1
      if ($0 ~ /^version[[:space:]]*=[[:space:]]*"/) dep_ver = quoted($0)
    }
    section ~ /dependencies\][[:space:]]*$/ &&
      /^[A-Za-z0-9_-]+[[:space:]]*=[[:space:]]*\{/ &&
      /path[[:space:]]*=/ {
      line = $0
      if (match(line, /version[[:space:]]*=[[:space:]]*"[^"]*"/)) {
        req = substr(line, RSTART, RLENGTH)
        name = $1
        printf "dep|%s|%s\n", name, quoted(req)
      }
    }
    END { flush_dep() }
  ' "$1"
}

# A floating requirement is satisfied when the target equals it or extends it
# by another dot component: '2.0' accepts 2.0.0 and 2.0.1; '2.0.0-rc.1'
# accepts exactly itself. A pre-release target satisfies no plain float:
# cargo excludes pre-releases from version ranges unless the requirement
# itself names one, so '2.0' does NOT accept 2.0.0-rc.1. This intentionally
# does not re-implement full cargo semver-range semantics; the requirements
# cargo-release writes are plain x[.y[.z]] floats or full versions.
float_satisfied() {
  local req="$1"
  [ "$version" = "$req" ] && return 0
  case "$version" in
  *-*) return 1 ;;
  "$req".*) return 0 ;;
  esac
  return 1
}

check_manifest() {
  local manifest="$1" rel="$2" kind name value pkg_seen=0
  while IFS='|' read -r kind name value; do
    case "$kind" in
    pkg)
      pkg_seen=1
      if [ "$name" = "literal" ] && [ "$value" != "$version" ]; then
        mismatches+=("$rel: [package] version is '$value'")
      fi
      ;;
    dep)
      case "$value" in
      =*)
        [ "$value" = "=$version" ] ||
          mismatches+=("$rel: dependency '$name' is pinned '$value'")
        ;;
      *)
        float_satisfied "$value" ||
          mismatches+=("$rel: dependency '$name' requires '$value', which '$version' does not satisfy")
        ;;
      esac
      ;;
    esac
  done < <(scan_manifest "$manifest")

  # Member manifests must say which version they are; a [package] table whose
  # version could not be classified would silently skip the check.
  if [ "$rel" != "Cargo.toml" ] && [ "$pkg_seen" -eq 0 ]; then
    mismatches+=("$rel: no [package] version found (neither literal nor workspace-inherited)")
  fi
}

# --- 2 + 3. the root manifest and every member -------------------------------
check_manifest "$root/Cargo.toml" "Cargo.toml"

members="$(awk '
  /^\[/ { section = $0; next }
  section == "[workspace]" {
    if ($0 ~ /^members[[:space:]]*=/) collecting = 1
    if (collecting)
      while (match($0, /"[^"]+"/)) {
        print substr($0, RSTART + 1, RLENGTH - 2)
        $0 = substr($0, RSTART + RLENGTH)
      }
    if (collecting && /\]/) collecting = 0
  }
' "$root/Cargo.toml")"
[ -n "$members" ] || die "no [workspace] members found in '$root/Cargo.toml'"

while IFS= read -r member; do
  manifest="$root/$member/Cargo.toml"
  if [ ! -f "$manifest" ]; then
    mismatches+=("$member/Cargo.toml: listed in [workspace] members but missing")
    continue
  fi
  check_manifest "$manifest" "$member/Cargo.toml"
done <<<"$members"

# --- verdict ------------------------------------------------------------------
if [ "${#mismatches[@]}" -gt 0 ]; then
  echo "version-consistency: ${#mismatches[@]} mismatch(es) against '$version':" >&2
  printf '  - %s\n' "${mismatches[@]}" >&2
  exit 1
fi

echo "version-consistency: every manifest agrees on '$version'"
