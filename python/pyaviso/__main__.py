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
