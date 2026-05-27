"""Shared bounded-wait helpers for e2e tests.

The sync helpers wrap blocking iterator reads and background-thread publishers so a
silently-failing publish or a missing notification surfaces as a timeout error within
seconds, not an indefinitely-hanging test. The async sibling is just `asyncio.wait_for`
applied to `__anext__()`; included here for symmetry.
"""

from __future__ import annotations

import asyncio
import contextlib
import queue
import threading
from collections.abc import Callable, Iterator
from typing import Any


def receive_within(iterator: Any, timeout: float) -> Any:
    """Pull one item from `iterator` with a wall-clock timeout.

    Runs `next(iterator)` on a daemon thread and waits up to `timeout` seconds. On
    timeout, calls `iterator.close()` (best-effort) and raises `TimeoutError`. Exceptions
    raised by the iterator surface verbatim.
    """
    box: queue.Queue[tuple[str, Any]] = queue.Queue(maxsize=1)

    def fetch() -> None:
        try:
            box.put(("ok", next(iterator)))
        except Exception as exc:
            box.put(("err", exc))

    thread = threading.Thread(target=fetch, daemon=True)
    thread.start()
    try:
        kind, value = box.get(timeout=timeout)
    except queue.Empty:
        with contextlib.suppress(Exception):
            iterator.close()
        raise TimeoutError(f"no notification within {timeout}s") from None
    if kind == "err":
        raise value
    return value


@contextlib.contextmanager
def background_publishes(target: Callable[[], None], timeout: float = 10.0) -> Iterator[None]:
    """Run `target()` on a daemon thread; re-raise any exception when the block exits.

    Use for fire-then-consume patterns where the test cannot afford to silently swallow
    a publisher-side error: the thread's exception is captured and raised at `__exit__`
    after waiting up to `timeout` seconds for the thread to finish.
    """
    excs: list[BaseException] = []

    def run() -> None:
        try:
            target()
        except Exception as exc:
            excs.append(exc)

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    try:
        yield
    finally:
        thread.join(timeout=timeout)
        if thread.is_alive():
            raise TimeoutError(f"background publisher did not complete within {timeout}s")
        if excs:
            raise excs[0]


async def receive_within_async(async_iterator: Any, timeout: float) -> Any:
    """`asyncio.wait_for(async_iterator.__anext__(), timeout)`. Included for symmetry."""
    return await asyncio.wait_for(async_iterator.__anext__(), timeout=timeout)
