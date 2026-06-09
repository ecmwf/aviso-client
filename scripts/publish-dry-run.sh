#!/usr/bin/env bash
#
# Ordered `cargo publish --dry-run` for the publishable crates.
#
# Publish order is dependency-topological: `finesse` first, then `aviso`, then
# the two independent leaves. A dry-run failure is classified, not blanket-
# tolerated:
#
#   - `finesse` has no workspace-internal dependencies, so any failure there is
#     a real packaging or metadata error: always fatal.
#   - A dependent crate may fail BEFORE THE FIRST PUBLISH with cargo's
#     unresolved-dependency error (its upstream is not on crates.io yet, and a
#     published manifest drops `path`, so the dry-run resolves against the
#     registry). Only that signature, naming one of our own crates, is reported
#     as a warning. Any other failure is fatal.
#
# Exits 0 when every crate either passed or hit only the expected first-publish
# resolution gap; prints a warning summary in that case.
#
# Usage: publish-dry-run.sh [crate ...]   (default: the publish-ordered set)

set -euo pipefail

crates=("$@")
[ "${#crates[@]}" -gt 0 ] || crates=(finesse aviso aviso-cli aviso-ffi)

# The first-publish resolution gap: cargo cannot find the (not yet published)
# upstream on the registry. Matches e.g.
#   error: no matching package named `aviso` found
#   error: failed to select a version for the requirement `aviso = "=2.0.0"`
# but only when the named package is exactly one of ours: the name must be
# followed by a non-name character, so e.g. an external `aviso-foo` does not
# ride on the `aviso` alternative. The `.` placeholders stand for cargo's
# backtick quoting, kept out of the pattern so the shell never reads them as
# command substitution.
unresolved_re='(no matching package named .(finesse|aviso-cli|aviso-ffi|aviso)[^0-9A-Za-z_-]|failed to select a version for the requirement .(finesse|aviso-cli|aviso-ffi|aviso)[^0-9A-Za-z_-])'

warned=()
for crate in "${crates[@]}"; do
  echo "--- cargo publish --dry-run -p $crate"
  log="$(mktemp)"
  if cargo publish --locked --dry-run -p "$crate" 2>&1 | tee "$log"; then
    rm -f "$log"
    continue
  fi

  if [ "$crate" = "finesse" ]; then
    rm -f "$log"
    echo "publish-dry-run: 'finesse' failed its dry-run; it has no internal" >&2
    echo "  dependencies, so this is a real packaging/metadata error, not" >&2
    echo "  first-publish index lag." >&2
    exit 1
  fi

  if grep -Eq "$unresolved_re" "$log"; then
    rm -f "$log"
    warned+=("$crate")
    echo "WARNING: '$crate' hit the first-publish resolution gap (its upstream"
    echo "  is not on crates.io yet); expected before the first release only."
    continue
  fi

  rm -f "$log"
  echo "publish-dry-run: '$crate' failed for a reason other than the" >&2
  echo "  first-publish resolution gap; see the cargo output above." >&2
  exit 1
done

if [ "${#warned[@]}" -gt 0 ]; then
  echo "publish-dry-run: finished WITH first-publish warnings: ${warned[*]}"
else
  echo "publish-dry-run: every crate passed"
fi
