#!/usr/bin/env bash

# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

# Exercise the publisher's actual shell command without extracting archives.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 - "$root" "${1:-}" <<'PY'
from __future__ import annotations

import io
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

root = Path(sys.argv[1])
workflow = (root / ".github/workflows/publish-pypi.yml").read_text(encoding="utf-8")
step = workflow.split("      - name: The sdist carries its canonical license files\n", 1)[1]
block = step.split("        run: |\n", 1)[1].split("\n\n", 1)[0]
command = "\n".join(line.removeprefix("          ") for line in block.splitlines())


def check(directory: Path, expected: bool, label: str) -> None:
    result = subprocess.run(
        ["bash", "-euo", "pipefail", "-c", command],
        cwd=directory, capture_output=True, text=True,
    )
    if (result.returncode == 0) != expected:
        raise AssertionError(f"{label}: {result.stdout}{result.stderr}")
    print(f"ok - {label}")


with tempfile.TemporaryDirectory() as temporary:
    directory = Path(temporary)
    (directory / "dist").mkdir()
    archive = directory / "dist/pyaviso-2.1.1.tar.gz"
    # pyaviso-2.1.1/NOTICE is valid; nested/NOTICE and other roots are not.
    cases = {
        "canonical files": (["LICENSE", "NOTICE"], True),
        "missing NOTICE": (["LICENSE"], False),
        "missing LICENSE": (["NOTICE"], False),
        "nested NOTICE": (["LICENSE", "nested/NOTICE"], False),
        "wrong root": (["../LICENSE", "../NOTICE"], False),
    }
    for label, (names, expected) in cases.items():
        with tarfile.open(archive, "w:gz") as package:
            for name in names:
                member = tarfile.TarInfo(f"pyaviso-2.1.1/{name}")
                member.size = 5
                package.addfile(member, io.BytesIO(b"legal"))
        check(directory, expected, label)
    archive.unlink()
    check(directory, False, "missing archive")

if sys.argv[2]:
    # An optional fresh artifact directory must contain dist/pyaviso-*.tar.gz.
    check(Path(sys.argv[2]).resolve(), True, "fresh sdist")
PY
