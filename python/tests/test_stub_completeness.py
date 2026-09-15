# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

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

import ast
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


def test_notification_str_is_exposed_in_both_stubs() -> None:
    assert "__str__" in pyaviso.Notification.__dict__
    for path in (_STUB_PATH, _STUB_PATH.with_name("_native.pyi")):
        module = ast.parse(path.read_text(encoding="utf-8"))
        notification = next(
            node
            for node in module.body
            if isinstance(node, ast.ClassDef) and node.name == "Notification"
        )
        method = next(
            node
            for node in notification.body
            if isinstance(node, ast.FunctionDef) and node.name == "__str__"
        )
        assert isinstance(method.returns, ast.Name) and method.returns.id == "str"


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
