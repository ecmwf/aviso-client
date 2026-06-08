# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Console-script entry point for the bundled ``aviso`` command.

``pip install pyaviso`` installs an ``aviso`` console script (declared under
``[project.scripts]`` in ``pyproject.toml``) that calls :func:`main`, which
runs the Rust CLI in-process through the compiled extension. ``python -m
pyaviso`` reaches the same entry point.
"""

from __future__ import annotations

import contextlib
import signal
import sys


def main() -> int:
    from pyaviso._native import _run_cli

    # Hand SIGINT to the bundled CLI so it behaves like the standalone `aviso`
    # binary: the in-process Rust client owns Ctrl+C for `listen` / `replay`,
    # and every other subcommand terminates promptly instead of Python holding
    # a pending KeyboardInterrupt until the blocking call returns. `signal`
    # only works on the main thread; the except below is the fallback path.
    with contextlib.suppress(ValueError):
        signal.signal(signal.SIGINT, signal.SIG_DFL)

    try:
        return _run_cli(sys.argv)
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
