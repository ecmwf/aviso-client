#!/usr/bin/env bash
#
# Assert the release invariant before any irreversible publish step.
#
# A bare semver tag (e.g. 2.0.0) can be pushed at an unreviewed commit, which
# bypasses branch protection on main (tag pushes are not gated by it). Before a
# publisher creates anything immutable (crates.io, PyPI, GitHub Release assets)
# it must prove, about the exact commit it is going to publish:
#
#   1. the checkout is that commit (HEAD == SHA), so what is verified here is
#      what gets packaged -- not a different tree,
#   2. the release tag actually names that commit (the tag peels to SHA),
#   3. the tag is bare semver and equals the [workspace.package] version,
#   4. the commit is reachable from main (it went through review and merged), and
#   5. the aggregate CI gate (ci-pass) is green for that exact commit, from the
#      trusted push-to-main run -- not a fork pull request or a manual dispatch.
#
# Any failure exits non-zero so the publisher stops before touching a registry.
#
# Inputs (environment; GitHub Actions provides these by default):
#   REF_NAME   the release tag                (default: $GITHUB_REF_NAME)
#   SHA        the commit the tag points at   (default: $GITHUB_SHA)
#   REPO       owner/repo                      (default: $GITHUB_REPOSITORY)
#   MANIFEST   workspace Cargo.toml            (default: the repo-root Cargo.toml)
#   GH_TOKEN   token with contents:read + actions:read (consumed by `gh`)
#
# Run from inside the checkout (the tag commit, with its tag ref available, as a
# tag-triggered `actions/checkout` leaves it). Requires the GitHub CLI `gh`.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

REF_NAME="${REF_NAME:-${GITHUB_REF_NAME:-}}"
SHA="${SHA:-${GITHUB_SHA:-}}"
REPO="${REPO:-${GITHUB_REPOSITORY:-}}"
MANIFEST="${MANIFEST:-$here/../Cargo.toml}"

die() {
  echo "release-invariant: $*" >&2
  exit 1
}

[ -n "$REF_NAME" ] || die "REF_NAME (or GITHUB_REF_NAME) is required"
[ -n "$SHA" ] || die "SHA (or GITHUB_SHA) is required"
[ -n "$REPO" ] || die "REPO (or GITHUB_REPOSITORY) is required"
command -v gh >/dev/null 2>&1 || die "the GitHub CLI 'gh' is required"

# Validate the values that get interpolated into API paths, so the script is
# safe even if reused with attacker-controlled environment outside Actions.
printf '%s' "$SHA" | grep -Eq '^([0-9a-f]{40}|[0-9a-f]{64})$' ||
  die "SHA '$SHA' is not a full hex commit id"
printf '%s' "$REPO" | grep -Eq '^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$' ||
  die "REPO '$REPO' is not in owner/name form"

# 1. the checkout is the commit we are about to verify and publish ------------
# Without this, every gate below could pass for a good SHA while the publisher
# packages a different tree (a tag-retarget race, or a miswired checkout).
head_sha="$(git rev-parse HEAD 2>/dev/null || true)"
[ -n "$head_sha" ] || die "not inside a git checkout; cannot bind the release to a commit"
[ "$head_sha" = "$SHA" ] ||
  die "checked-out HEAD ($head_sha) is not the release commit ($SHA)"

# 2. the release tag names that commit ---------------------------------------
# Peels lightweight and annotated tags alike to the underlying commit.
tag_sha="$(git rev-parse -q --verify "refs/tags/$REF_NAME^{commit}" 2>/dev/null || true)"
[ -n "$tag_sha" ] ||
  die "tag '$REF_NAME' is not present in the checkout (fetch the release tag first)"
[ "$tag_sha" = "$SHA" ] ||
  die "tag '$REF_NAME' points at $tag_sha, not the release commit $SHA"

# 3. tag == workspace version ------------------------------------------------
# Bare semver, optionally a pre-release (e.g. 2.0.0-rc.1) or build metadata.
semver_re='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'
printf '%s' "$REF_NAME" | grep -Eq "$semver_re" ||
  die "tag '$REF_NAME' is not a bare semver release tag"

ws_version="$("$here/ws-version.sh" "$MANIFEST")"
[ "$REF_NAME" = "$ws_version" ] ||
  die "tag '$REF_NAME' does not match workspace version '$ws_version'"
echo "ok: tag '$REF_NAME' names the checked-out commit and matches the workspace version"

# 4. commit reachable from main ----------------------------------------------
# compare main...$SHA, viewed from main as the base:
#   identical -> $SHA is the current main tip
#   behind    -> $SHA is an ancestor of main (merged earlier)
#   ahead     -> $SHA carries commits main does not have (never merged)
#   diverged  -> both have unique commits
# Only the first two mean "reachable from main".
compare_status="$(gh api "repos/$REPO/compare/main...$SHA" --jq '.status' 2>/dev/null || true)"
case "$compare_status" in
behind | identical)
  echo "ok: $SHA is reachable from main (compare: $compare_status)"
  ;;
"")
  die "could not compare $SHA against main (no API response for repos/$REPO)"
  ;;
*)
  die "commit $SHA is not reachable from main (compare: $compare_status)"
  ;;
esac

# 5. ci-pass green for this commit, from the trusted push-to-main run ---------
# Bind to the CI workflow run for this exact commit, triggered by a push to
# main (the branch-protected path), not a fork PR or a workflow_dispatch.
runs_query="head_sha=$SHA&event=push&branch=main&status=completed&per_page=100"
run_summary="$(gh api "repos/$REPO/actions/workflows/ci.yml/runs?$runs_query" \
  --jq '.workflow_runs | sort_by(.run_started_at) | last
        | if . == null then "" else "\(.id)\t\(.conclusion)" end')"
[ -n "$run_summary" ] ||
  die "no completed push-to-main CI run found for $SHA (was it merged and did CI run?)"

run_id="${run_summary%%$'\t'*}"
run_conclusion="${run_summary##*$'\t'}"
[ "$run_conclusion" = "success" ] ||
  die "the CI run ($run_id) for $SHA concluded '$run_conclusion', not success"

# The aggregate gate is the ci-pass job inside that run. Accept the latest
# successful attempt (a green re-run is a valid pass); require at least one.
ci_pass_ok="$(gh api "repos/$REPO/actions/runs/$run_id/jobs?per_page=100" \
  --jq '[.jobs[] | select(.name == "ci-pass" and .conclusion == "success")] | length')"
[ "${ci_pass_ok:-0}" -ge 1 ] ||
  die "ci-pass did not succeed in CI run $run_id for $SHA"
echo "ok: ci-pass is green for $SHA (CI run $run_id)"

echo "release invariant satisfied for $REF_NAME ($SHA)"
