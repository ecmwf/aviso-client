# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Helpers shared by the example scripts.

Each example file imports from this module so the body of every example
stays focused on the scenario rather than repeating boilerplate:

- ``require_env()``: validates the two environment variables every script
  needs (`AVISO_BASE_URL` plus credentials), with a clear error message
  instead of a confusing ``KeyError``.
- ``break_after(iterator, n)``: yields up to ``n`` notifications from a
  listener, then exits cleanly. Examples use this so they terminate in
  bounded time (useful for the fact-check harness and for users who copy
  the example and want to see it complete).
- ``temp_dir()``: a contextmanager around ``tempfile.TemporaryDirectory``
  that returns a ``pathlib.Path``; trigger side-effects (log files,
  state files) write inside it so parallel runs do not collide.
"""

from __future__ import annotations

import contextlib
import os
import sys
import tempfile
from collections.abc import Iterable, Iterator
from pathlib import Path
from typing import TypeVar

T = TypeVar("T")


def require_env() -> str:
    base_url = os.environ.get("AVISO_BASE_URL")
    has_token = bool(os.environ.get("AVISO_TOKEN"))
    has_basic = bool(os.environ.get("AVISO_USERNAME")) and bool(os.environ.get("AVISO_PASSWORD"))
    if not base_url:
        sys.stderr.write(
            "set AVISO_BASE_URL to the aviso-server URL "
            "(plus AVISO_TOKEN, or AVISO_USERNAME and AVISO_PASSWORD)\n"
        )
        sys.exit(2)
    if not (has_token or has_basic):
        sys.stderr.write(
            "set AVISO_TOKEN, or both AVISO_USERNAME and AVISO_PASSWORD, alongside AVISO_BASE_URL\n"
        )
        sys.exit(2)
    return base_url


def break_after(iterator: Iterable[T], n: int) -> Iterator[T]:
    """Yield up to ``n`` items from an iterator, then stop.

    The examples use this so they terminate in bounded time. A real
    long-running listener would not have this cap; the docstring of each
    example file calls out where to remove the call for production use.
    """
    for i, item in enumerate(iterator, start=1):
        yield item
        if i >= n:
            return


@contextlib.contextmanager
def temp_dir(prefix: str = "aviso-example-") -> Iterator[Path]:
    """Yield a temporary directory as a ``pathlib.Path``; clean up on exit."""
    with tempfile.TemporaryDirectory(prefix=prefix) as raw:
        yield Path(raw)
