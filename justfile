# Release automation for aviso-client.
#
# The justfile PREPARES and TAGS a release locally; CI does every actual publish
# on the pushed tag. See plans/release-plan.md for the full process, decisions,
# and the oracle-reviewed safety model.
#
# Requires `just` and `cargo-release`:  cargo install just cargo-release
# The Python preflight steps additionally require `uv` (https://docs.astral.sh/uv/).

# crates.io crates in dependency (publish) order.
crates := "finesse aviso aviso-cli aviso-ffi"

# List available recipes.
default:
    @just --list

# Print the version from the [workspace.package] section (section-scoped so it
# does not match a `version` key in another table or depend on file ordering).
_ws-version:
    @awk '/^\[/{s=$0} s=="[workspace.package]" && /^version[[:space:]]*=/{if(match($0,/"[^"]+"/)){print substr($0,RSTART+1,RLENGTH-2);exit}}' Cargo.toml

# Dry-run gate before a release (publishes nothing); pass the target version.
release-preflight version:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> release-preflight {{version}}"

    actual=$(just _ws-version)
    if [ "$actual" != "{{version}}" ]; then
        echo "ERROR: workspace version is '$actual', expected '{{version}}'." >&2
        echo "Run 'just release-version {{version}}' first (or fix the target)." >&2
        exit 1
    fi

    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace --all-targets

    for c in {{crates}}; do
        echo "--- cargo package --list -p $c"
        cargo package --locked -p "$c" --list >/dev/null
    done

    echo "==> cargo publish --dry-run (ordered)"
    dry_failures=()
    for c in {{crates}}; do
        echo "--- $c"
        if ! cargo publish --locked --dry-run -p "$c"; then
            dry_failures+=("$c")
        fi
    done

    # finesse has no workspace-internal dependencies, so a dry-run failure there
    # is a real packaging/metadata error, not the first-publish resolution case.
    if printf '%s\n' "${dry_failures[@]:-}" | grep -qx finesse; then
        echo "ERROR: 'finesse' failed cargo publish --dry-run — it has no internal deps," >&2
        echo "  so this is a real packaging/metadata error, not first-publish index lag." >&2
        exit 1
    fi
    if [ "${#dry_failures[@]}" -gt 0 ]; then
        echo "WARNING: cargo publish --dry-run failed for: ${dry_failures[*]}"
        echo "  Expected on a FIRST publish for crates whose upstreams are not yet on"
        echo "  crates.io; investigate any UNEXPECTED failure (real metadata error)."
        echo "  See plans/release-plan.md section 13 (B2) and the 2.0.0-rc.1 rehearsal."
    fi

    if command -v uv >/dev/null 2>&1; then
        echo "==> Python wheel + sdist + twine check"
        rm -rf dist && mkdir -p dist
        uv run maturin build --release --out dist
        uv run maturin sdist --out dist
        uv run --with twine twine check dist/*
    else
        echo "==> SKIP Python checks (uv not installed)"
    fi

    if [ "${#dry_failures[@]}" -gt 0 ]; then
        echo "==> preflight finished for {{version}} (WITH dry-run warnings above)"
    else
        echo "==> preflight OK for {{version}}"
    fi

# Bump the whole workspace and internal pins to <version> (no tag, no push).
release-version version:
    cargo release version --execute {{version}}

# Tag the current commit as the bare <version> and print the push command.
release-tag version:
    #!/usr/bin/env bash
    set -euo pipefail
    actual=$(just _ws-version)
    if [ "$actual" != "{{version}}" ]; then
        echo "ERROR: workspace version is '$actual', not '{{version}}'." >&2
        echo "Run 'just release-version {{version}}' and merge it first." >&2
        exit 1
    fi
    git tag -a "{{version}}" -m "Release {{version}}"
    echo "Tag {{version}} created. Review, then push to trigger the release:"
    echo "    git push origin {{version}}"

# Launch the CI dry-run paths (crates.io --dry-run, TestPyPI) via gh.
publish-dry:
    gh workflow run publish-crates.yml -f dry_run=true
    gh workflow run publish-pypi.yml -f use_test_pypi=true
