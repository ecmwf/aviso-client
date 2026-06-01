"""The bundled `aviso` console command runs the Rust CLI in-process.

`pyaviso._native._run_cli` backs the `aviso` console script declared in
`pyproject.toml`. These cases pin the exit-code contract for the argument-parsing
paths (which return before any network or async runtime is involved).
"""

from __future__ import annotations

from pyaviso._native import _run_cli


def test_version_returns_zero() -> None:
    assert _run_cli(["aviso", "--version"]) == 0


def test_help_returns_zero() -> None:
    assert _run_cli(["aviso", "--help"]) == 0


def test_unknown_subcommand_returns_two() -> None:
    assert _run_cli(["aviso", "definitely-not-a-subcommand"]) == 2


def test_missing_required_argument_returns_two() -> None:
    assert _run_cli(["aviso", "notify"]) == 2
