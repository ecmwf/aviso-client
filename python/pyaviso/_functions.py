# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Calling ``Trigger.function`` functions.

The built-in triggers run inside the library, before a notification reaches
Python. A function trigger runs here instead, in the thread that reads the
notification or on its event loop, so the function needs no synchronisation
and may be a coroutine. ``listen`` wraps its iterator in
``FunctionTriggerIterator`` when its triggers include a function; ``listen_many``
calls ``call_functions`` for each listener's notifications.
"""

from __future__ import annotations

import asyncio
import inspect
import logging
from collections.abc import Callable
from typing import Any

from pyaviso._native import Notification, TriggerError

_log = logging.getLogger("pyaviso.listen")

# How the native side passes a function trigger: (func, retries, required, label).
_Function = tuple[Callable[[Notification], object], int, bool, "str | None"]


def _function_name(func: Callable[[Notification], object], label: str | None) -> str:
    return label or getattr(func, "__qualname__", None) or repr(func)


def _trigger_error(
    func: Callable[[Notification], object], label: str | None, attempts: int, cause: BaseException
) -> TriggerError:
    name = _function_name(func, label)
    error = TriggerError(f"function trigger {name} failed after {attempts} attempt(s): {cause!r}")
    error.trigger_kind = "function"
    error.error_kind = "raised"
    # Every attribute TriggerError declares is present; those that describe
    # built-in triggers do not apply to a function and are None.
    for field in (
        "path",
        "exit_code",
        "stderr_tail",
        "status",
        "body_tail",
        "reason",
        "timeout_seconds",
        "context",
        "field",
        "template_kind",
    ):
        setattr(error, field, None)
    return error


def _optional_failure(
    func: Callable[[Notification], object], label: str | None, cause: BaseException
) -> None:
    _log.warning(
        "optional function trigger %s failed; continuing: %r",
        _function_name(func, label),
        cause,
    )


def call_functions(notification: Notification, functions: list[_Function]) -> None:
    """Calls each function trigger with the notification, in order.

    A function that raises is called again up to ``retries`` times. If it
    still fails, a required one raises ``TriggerError``; an optional one is
    logged and skipped.
    """
    for func, retries, required, label in functions:
        failure: BaseException | None = None
        for _ in range(retries + 1):
            try:
                result = func(notification)
            # reason: the function is the caller's code, so any exception it
            # raises is its failure; it is retried, then raised as a
            # TriggerError or, for an optional function, logged.
            except Exception as e:
                failure = e
                continue
            if inspect.isawaitable(result):
                # Nothing here can await it. listen() refuses async functions
                # up front; this catches a plain function that returns an
                # awaitable anyway. It is a programming error, not a failure
                # of one notification, so it stops the loop whatever on_error
                # says.
                if inspect.iscoroutine(result):
                    result.close()
                raise TypeError(
                    f"{_function_name(func, label)} returned an awaitable; an async "
                    "function needs AsyncAvisoClient, which awaits it. With "
                    "AvisoClient, pass a plain function."
                )
            failure = None
            break
        if failure is None:
            continue
        if required:
            raise _trigger_error(func, label, retries + 1, failure) from failure
        _optional_failure(func, label, failure)


async def acall_functions(notification: Notification, functions: list[_Function]) -> None:
    """As ``call_functions``, awaiting functions that return an awaitable."""
    for func, retries, required, label in functions:
        failure: BaseException | None = None
        for _ in range(retries + 1):
            try:
                result = func(notification)
                if inspect.isawaitable(result):
                    await result
            # reason: the function is the caller's code, so any exception it
            # raises is its failure; it is retried, then raised as a
            # TriggerError or, for an optional function, logged.
            except Exception as e:
                failure = e
                continue
            failure = None
            break
        if failure is None:
            continue
        if required:
            raise _trigger_error(func, label, retries + 1, failure) from failure
        _optional_failure(func, label, failure)


class FunctionTriggerIterator:
    """What ``AvisoClient.listen`` returns when its triggers include
    ``Trigger.function``: the same iterator, calling the functions as each
    notification is read. A required function that fails raises
    ``TriggerError`` from the loop."""

    # reason: inner is the native iterator this wraps; its type is declared in
    # the public stubs.
    def __init__(self, inner: Any, functions: list[_Function]) -> None:
        self._inner = inner
        self._functions = functions

    def __iter__(self) -> FunctionTriggerIterator:
        return self

    def __next__(self) -> Notification:
        notification = next(self._inner)
        try:
            call_functions(notification, self._functions)
        except BaseException:
            self.close()
            raise
        return notification

    def run(self) -> None:
        """Reads every notification until the stream ends, calling the
        triggers, then closes it."""
        try:
            for _ in self:
                pass
        finally:
            self.close()

    def close(self) -> None:
        """Closes the listener. Safe to call more than once."""
        self._inner.close()

    def __repr__(self) -> str:
        return f"FunctionTriggerIterator(functions={len(self._functions)})"

    def __enter__(self) -> FunctionTriggerIterator:
        return self

    def __exit__(self, *exc: object) -> bool:
        self.close()
        return False


class AsyncFunctionTriggerIterator:
    """As ``FunctionTriggerIterator``, for ``AsyncAvisoClient.listen``."""

    # reason: inner is the native iterator this wraps; its type is declared in
    # the public stubs.
    def __init__(self, inner: Any, functions: list[_Function]) -> None:
        self._inner = inner
        self._functions = functions

    def __aiter__(self) -> AsyncFunctionTriggerIterator:
        return self

    async def __anext__(self) -> Notification:
        notification = await self._inner.__anext__()
        try:
            await acall_functions(notification, self._functions)
        except asyncio.CancelledError:
            raise  # as for a cancelled read: the listener stays open
        except BaseException:
            await self.aclose()
            raise
        return notification

    async def run(self) -> None:
        """Reads every notification until the stream ends, calling (and
        awaiting) the triggers, then closes it."""
        try:
            async for _ in self:
                pass
        finally:
            await self.aclose()

    async def aclose(self) -> None:
        """Closes the listener. Safe to call more than once."""
        await self._inner.aclose()

    def __repr__(self) -> str:
        return f"AsyncFunctionTriggerIterator(functions={len(self._functions)})"

    async def __aenter__(self) -> AsyncFunctionTriggerIterator:
        return self

    async def __aexit__(self, *exc: object) -> bool:
        await self.aclose()
        return False
