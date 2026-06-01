"""Stub-completeness gate: every public name in pyaviso.__all__ has a stub.

The stub file ``python/pyaviso/__init__.pyi`` describes the public surface
that ``ty check`` enforces; if a name lands in ``__all__`` without a
corresponding stub entry, the gate fails and the binding maintainer must
either remove it from ``__all__`` or add a stub.

The test reads the stub source as text and looks for the symbol name as
a top-level definition or assignment. It is intentionally simple; a
stricter signature-shape comparison lives in a separate file once
``inspect.signature`` is wired against the PyO3 surface.
"""

from __future__ import annotations

import pathlib
import re

import pyaviso

_STUB_PATH = pathlib.Path(pyaviso.__file__).with_suffix(".pyi")


def _stub_source() -> str:
    return _STUB_PATH.read_text(encoding="utf-8")


def _stub_has_symbol(source: str, name: str) -> bool:
    patterns = [
        rf"^{re.escape(name)}\s*:",
        rf"^class\s+{re.escape(name)}\b",
        rf"^def\s+{re.escape(name)}\b",
        rf"^{re.escape(name)}\s*=",
    ]
    return any(re.search(pattern, source, re.MULTILINE) for pattern in patterns)


def test_stub_file_exists() -> None:
    assert _STUB_PATH.is_file(), f"expected stub at {_STUB_PATH}"


def test_every_public_name_appears_in_the_stub() -> None:
    source = _stub_source()
    missing = [name for name in pyaviso.__all__ if not _stub_has_symbol(source, name)]
    assert not missing, (
        f"these names are in pyaviso.__all__ but absent from {_STUB_PATH.name}: {missing}. "
        "Either add a stub entry or drop the name from __all__."
    )


def test_stub_lists_the_same_all() -> None:
    source = _stub_source()
    match = re.search(r"__all__\s*=\s*\[(?P<body>.+?)\]", source, re.DOTALL)
    assert match, "stub file must declare __all__"
    stub_names = set(re.findall(r"\"([A-Za-z_][A-Za-z0-9_]*)\"", match.group("body")))
    assert stub_names == set(pyaviso.__all__), (
        f"stub __all__ {sorted(stub_names)} does not match runtime "
        f"__all__ {sorted(pyaviso.__all__)}"
    )
