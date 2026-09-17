# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

# Release automation for aviso-client.
#
# The justfile PREPARES and TAGS a release locally; CI does every actual publish
# on the pushed tag.
#
# Requires `just` and `cargo-release`:  cargo install just cargo-release
# The Python preflight steps additionally require `uv` (https://docs.astral.sh/uv/).

# crates.io crates in dependency (publish) order.
crates := "finesse aviso aviso-cli aviso-ffi"

# List available recipes.
default:
    @just --list

# Print the [workspace.package] version. Delegates to scripts/ws-version.sh so
# the manifest reader has one home, shared with the release-invariant check.
_ws-version:
    @scripts/ws-version.sh

# Dry-run gate before a release (publishes nothing); pass the target version.
# The version-consistency and ordered-dry-run checks live in scripts/ (one
# home, shared with the release-preflight CI workflow and unit-tested there).
release-preflight version:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> release-preflight {{version}}"

    scripts/version-consistency.sh "{{version}}"

    cargo fmt --all -- --check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace --all-targets

    for c in {{crates}}; do
        echo "--- cargo package --list -p $c"
        cargo package --locked -p "$c" --list >/dev/null
    done

    echo "==> cargo publish --dry-run (ordered)"
    scripts/publish-dry-run.sh {{crates}}

    if command -v uv >/dev/null 2>&1; then
        echo "==> Python wheel + sdist + twine check"
        rm -rf dist && mkdir -p dist
        uv run --no-project --with maturin maturin build --release --locked --out dist
        uv run --no-project --with maturin maturin sdist --out dist
        uv run --no-project --with twine twine check dist/*
    else
        echo "==> SKIP Python checks (uv not installed)"
    fi

    echo "==> preflight finished for {{version}} (any first-publish dry-run warnings are listed above)"

# Bump the whole workspace and internal pins to <version> (no tag, no push).
release-version version:
    cargo release version --execute {{version}}

# Tag the current commit as the bare <version> and print the push command.
release-tag version:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "$(git status --porcelain)" ]; then
        echo "ERROR: the working tree has uncommitted changes." >&2
        echo "The version lives in the working tree, so tagging now could point the tag" >&2
        echo "at a commit without the bump. Commit and merge the bump first, then tag." >&2
        exit 1
    fi
    actual=$(just _ws-version)
    if [ "$actual" != "{{version}}" ]; then
        echo "ERROR: workspace version is '$actual', not '{{version}}'." >&2
        echo "Run 'just release-version {{version}}' and merge it first." >&2
        exit 1
    fi
    git tag -a "{{version}}" -m "Release {{version}}"
    echo "Tag {{version}} created. Check the diff, then push to trigger the release:"
    echo "    git push origin {{version}}"

# Launch the CI crates.io publish dry-run via gh. The PyPI build path is
# rehearsed by the Release Preflight workflow instead.
publish-dry:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v gh >/dev/null 2>&1; then
        echo "ERROR: the GitHub CLI 'gh' is required (https://cli.github.com) and must be authenticated." >&2
        exit 1
    fi
    gh workflow run publish-crates.yml -f dry_run=true

# Install the aviso C library (cargo-c) into <prefix> with the standard
# `.a/.so/.dylib` + `.pc` + headers layout. Needs cargo-c (cargo install cargo-c).
# `--libdir` is pinned so the layout is fixed (some distros default to multiarch).
ffi-cinstall prefix:
    cargo cinstall --locked -p aviso-ffi --release \
        --prefix="{{prefix}}" --libdir="{{prefix}}/lib"
