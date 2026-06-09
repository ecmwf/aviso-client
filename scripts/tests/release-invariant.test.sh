#!/usr/bin/env bash
#
# Unit tests for release-invariant.sh.
#
# Two things are faked: a git checkout (a throwaway repo with a known HEAD and
# an annotated release tag) and the GitHub API (a `gh` shim that applies the
# real --jq filter to raw JSON fixtures, so the actual jq expressions -- newest
# run, null-on-empty, ci-pass job select -- are exercised, not stubbed out).
# Every invariant is then checked offline, including the failure paths that must
# never pass on absence. Requires git, gh-shim's jq, and bash.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script="$root/scripts/release-invariant.sh"
command -v jq >/dev/null 2>&1 || {
  echo "these tests need jq on PATH" >&2
  exit 1
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Fake `gh`: route by endpoint to a raw JSON fixture and apply the real --jq.
mkdir -p "$tmp/bin"
cat >"$tmp/bin/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
endpoint="" filter=""
args=("$@")
i=0
while [ "$i" -lt "${#args[@]}" ]; do
  case "${args[$i]}" in
    --jq) i=$((i + 1)); filter="${args[$i]}" ;;
    -*|api) : ;;
    *) [ -z "$endpoint" ] && endpoint="${args[$i]}" ;;
  esac
  i=$((i + 1))
done
case "$endpoint" in
  */compare/*)                      fixture="$GH_FIXTURE/compare.json" ;;
  */actions/runs/*/jobs*)           fixture="$GH_FIXTURE/jobs.json" ;;
  */actions/workflows/ci.yml/runs*) fixture="$GH_FIXTURE/runs.json" ;;
  *) echo "fake gh: unexpected endpoint: $endpoint" >&2; exit 1 ;;
esac
jq -r "$filter" "$fixture"
EOF
chmod +x "$tmp/bin/gh"
export PATH="$tmp/bin:$PATH"

# A throwaway git checkout: one commit carrying a known workspace version, with
# both a matching and a deliberately mismatched annotated tag on that commit.
repo="$tmp/repo"
git init -q "$repo"
git -C "$repo" config user.email tester@example.invalid
git -C "$repo" config user.name tester
cat >"$repo/Cargo.toml" <<'EOF'
[workspace.package]
version = "9.9.9"

[package]
version = "0.0.0"
EOF
git -C "$repo" add Cargo.toml
git -C "$repo" commit -qm "init"
git -C "$repo" tag -a 9.9.9 -m "Release 9.9.9"
git -C "$repo" tag -a 8.8.8 -m "Release 8.8.8"
HEAD_SHA="$(git -C "$repo" rev-parse HEAD)"

# A second checkout whose tag points at an earlier commit than HEAD, to prove
# the tag-names-the-commit invariant.
repo2="$tmp/repo2"
git init -q "$repo2"
git -C "$repo2" config user.email tester@example.invalid
git -C "$repo2" config user.name tester
cat >"$repo2/Cargo.toml" <<'EOF'
[workspace.package]
version = "9.9.9"
EOF
git -C "$repo2" add Cargo.toml
git -C "$repo2" commit -qm "first"
git -C "$repo2" tag -a 9.9.9 -m "Release 9.9.9 (stale target)"
git -C "$repo2" commit -q --allow-empty -m "second"
HEAD_SHA2="$(git -C "$repo2" rev-parse HEAD)"

# A syntactically valid commit id for cases that must fail before any git read.
VALID_SHA="0123456789abcdef0123456789abcdef01234567"

# Raw JSON fixtures, written verbatim so the gh shim's real jq does the work.
runs_ok='{"workflow_runs":[
  {"id":100,"run_started_at":"2024-01-01T00:00:00Z","conclusion":"failure"},
  {"id":456,"run_started_at":"2024-06-01T00:00:00Z","conclusion":"success"}]}'
runs_failed='{"workflow_runs":[{"id":789,"run_started_at":"2024-01-01T00:00:00Z","conclusion":"failure"}]}'
runs_none='{"workflow_runs":[]}'
jobs_pass='{"jobs":[{"name":"rust","conclusion":"success"},{"name":"ci-pass","conclusion":"success"}]}'
jobs_redgate='{"jobs":[{"name":"rust","conclusion":"success"},{"name":"ci-pass","conclusion":"failure"}]}'

make_fixture() { # name compare_json runs_json jobs_json
  local d="$tmp/fix/$1"
  mkdir -p "$d"
  printf '%s' "$2" >"$d/compare.json"
  printf '%s' "$3" >"$d/runs.json"
  printf '%s' "$4" >"$d/jobs.json"
  printf '%s' "$d"
}

green="$(make_fixture green '{"status":"behind"}' "$runs_ok" "$jobs_pass")"

passed=0
failed=0

check() { # name expected_rc cwd sha ref fixture [repo]
  local name="$1" expected="$2" cwd="$3" sha="$4" ref="$5" fixture="$6"
  local repo="${7:-ecmwf/aviso-client}"
  local rc=0
  (
    cd "$cwd"
    MANIFEST="$cwd/Cargo.toml" REF_NAME="$ref" SHA="$sha" \
      REPO="$repo" GH_FIXTURE="$fixture" \
      bash "$script"
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

# Happy path: matching version, reachable, ci-pass green (newest run wins).
check "passes when every invariant holds" 0 "$repo" "$HEAD_SHA" 9.9.9 "$green"
check "passes when the commit is the main tip" 0 "$repo" "$HEAD_SHA" 9.9.9 \
  "$(make_fixture identical '{"status":"identical"}' "$runs_ok" "$jobs_pass")"

# Input validation (fails before any git or API read).
check "fails on a non-hex SHA" 1 "$repo" "deadbeef" 9.9.9 "$green"
check "fails on a malformed REPO" 1 "$repo" "$HEAD_SHA" 9.9.9 "$green" "not-a-repo"

# Checkout/commit binding.
check "fails when HEAD is not the release commit" 1 "$repo" "$VALID_SHA" 9.9.9 "$green"
check "fails when the tag names a different commit than HEAD" 1 \
  "$repo2" "$HEAD_SHA2" 9.9.9 "$green"
check "fails when the release tag is absent from the checkout" 1 \
  "$repo" "$HEAD_SHA" 7.7.7 "$green"

# Tag / version invariant.
check "fails on a tag that does not match the workspace version" 1 \
  "$repo" "$HEAD_SHA" 8.8.8 "$green"

# Reachability invariant.
check "fails when the commit is ahead of main (never merged)" 1 \
  "$repo" "$HEAD_SHA" 9.9.9 "$(make_fixture ahead '{"status":"ahead"}' "$runs_ok" "$jobs_pass")"
check "fails when history has diverged from main" 1 \
  "$repo" "$HEAD_SHA" 9.9.9 "$(make_fixture diverged '{"status":"diverged"}' "$runs_ok" "$jobs_pass")"
check "fails when the compare API returns nothing" 1 \
  "$repo" "$HEAD_SHA" 9.9.9 "$(make_fixture nocompare '' "$runs_ok" "$jobs_pass")"

# ci-pass invariant.
check "fails when no push-to-main CI run exists for the commit" 1 \
  "$repo" "$HEAD_SHA" 9.9.9 "$(make_fixture norun '{"status":"behind"}' "$runs_none" "$jobs_pass")"
check "fails when the CI run did not conclude success" 1 \
  "$repo" "$HEAD_SHA" 9.9.9 "$(make_fixture redrun '{"status":"behind"}' "$runs_failed" "$jobs_pass")"
check "fails when the ci-pass job is absent or not green" 1 \
  "$repo" "$HEAD_SHA" 9.9.9 "$(make_fixture nocipass '{"status":"behind"}' "$runs_ok" "$jobs_redgate")"

echo "----"
echo "passed: $passed  failed: $failed"
[ "$failed" -eq 0 ]
