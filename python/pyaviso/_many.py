# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Delivering notifications from several listeners through one loop.

The native side opens the watches and hands out raw items: ``(name,
Notification)``, or ``(name, exception)`` when a listener fails. This module
decides what happens next, in the caller's thread or on its event loop:

- each listener's ``Trigger.function`` functions are called with its
  notifications, after the built-in triggers and before the notification is
  yielded;
- ``on_error`` decides what a failure does: ``"raise"`` stops everything,
  ``"continue"`` drops the failed listener (or skips the notification, for a
  failed function) and carries on, and a function is asked;
- ``run()`` reads everything without a loop of the caller's own.

It is in Python because it calls back into the caller's code: functions that
may be coroutines, and an ``on_error`` that may raise.
"""

from __future__ import annotations

import asyncio
import inspect
import logging
from collections.abc import Callable
from dataclasses import dataclass
from enum import Enum
from typing import Any, Literal

from pyaviso._functions import _Function, acall_functions, call_functions
from pyaviso._native import AvisoError, TriggerError

_log = logging.getLogger("pyaviso.listen")

# on_error is either one of two fixed words or a function, so it stays a
# Literal union rather than an Enum: callers pass plain strings.
OnError = Literal["raise", "continue"] | Callable[[str, BaseException], object]


class ListenFailureKind(str, Enum):
    """What failed: a listener, or a ``Trigger.function`` for one notification."""

    LISTENER = "listener"
    TRIGGER = "trigger"


@dataclass(frozen=True, slots=True)
class ListenFailure:
    """One failure seen by a ``listen_many`` iterator.

    ``kind`` is ``ListenFailureKind.LISTENER`` (equal to ``"listener"``) when a
    listener stopped (a bad filter, a missing permission, the replay limit), or
    ``ListenFailureKind.TRIGGER`` (``"trigger"``) when a ``Trigger.function``
    raised for one notification.
    """

    listener: str
    kind: ListenFailureKind
    error: BaseException


def _named(name: str, error: AvisoError) -> AvisoError:
    """Puts the listener's name on the error, and in front of its message."""
    error.listener = name
    prefix = f"listener '{name}': "
    if error.args and isinstance(error.args[0], str) and not error.args[0].startswith(prefix):
        error.args = (prefix + error.args[0], *error.args[1:])
    return error


class _Policy:
    """What ``listen_many`` does with failures, shared by both iterators."""

    def __init__(self, on_error: OnError, count: int) -> None:
        self.on_error = on_error
        self.count = count
        self.failures: list[ListenFailure] = []
        self.failed_listeners: set[str] = set()

    def record(self, failure: ListenFailure) -> bool:
        """Records a failure. Returns True when it must be raised now."""
        self.failures.append(failure)
        if failure.kind is ListenFailureKind.LISTENER:
            self.failed_listeners.add(failure.listener)
        if self.on_error == "raise":
            return True
        if callable(self.on_error):
            return False  # the caller's function reports it
        _log.warning(
            "%s (%s)",
            failure.error,
            "listener stopped; the other listeners continue"
            if failure.kind is ListenFailureKind.LISTENER
            else "notification skipped; the listener continues",
        )
        return False

    def ask(self, failure: ListenFailure) -> None:
        """Hands the failure to an ``on_error`` function, if there is one."""
        if callable(self.on_error):
            result = self.on_error(failure.listener, failure.error)
            if inspect.isawaitable(result):
                if inspect.iscoroutine(result):
                    result.close()
                raise TypeError(
                    "on_error returned an awaitable; an async function needs "
                    "AsyncAvisoClient, which awaits it"
                )

    def check_all_failed(self) -> None:
        """Raises when the loop ended because every listener failed. Under
        "raise" the first failure was already raised, so there is nothing
        to add."""
        if self.on_error == "raise":
            return
        if self.count and len(self.failed_listeners) == self.count:
            details = "; ".join(
                str(f.error) for f in self.failures if f.kind is ListenFailureKind.LISTENER
            )
            error = AvisoError(f"every listener failed: {details}")
            error.failures = list(self.failures)
            raise error


class MultiNotificationIterator:
    """Notifications from several listeners, as ``(name, notification)`` pairs.

    Returned by ``AvisoClient.listen_many``. Use it in a ``with`` block and a
    ``for`` loop, or call ``run()`` to read everything without a loop.
    """

    # reason: raw is the private native iterator; its items are typed in the
    # public stubs, not here.
    def __init__(
        self, raw: Any, functions: dict[str, list[_Function]], on_error: OnError, count: int
    ) -> None:
        self._raw = raw
        self._functions = functions
        self._policy = _Policy(on_error, count)
        self._closed = False
        self._stopped = False

    @property
    def errors(self) -> list[ListenFailure]:
        """Every failure so far, in order. Empty with ``on_error="raise"``
        until the one that was raised."""
        return list(self._policy.failures)

    def __iter__(self) -> MultiNotificationIterator:
        return self

    def __next__(self) -> tuple[str, Any]:
        if self._stopped:
            raise StopIteration
        try:
            return self._next()
        except BaseException:
            # Whatever ended the loop (the end, a raised failure, Ctrl+C),
            # the listeners are closed and the next call just stops.
            self._stopped = True
            self.close()
            raise

    def _next(self) -> tuple[str, Any]:
        while True:
            try:
                name, item = next(self._raw)
            except StopIteration:
                self.close()
                self._policy.check_all_failed()
                raise
            if isinstance(item, BaseException):
                self._fail(ListenFailure(name, ListenFailureKind.LISTENER, _named(name, item)))
                continue
            try:
                call_functions(item, self._functions.get(name, []))
            except TriggerError as e:
                self._fail(ListenFailure(name, ListenFailureKind.TRIGGER, _named(name, e)))
                continue
            return name, item

    def _fail(self, failure: ListenFailure) -> None:
        if self._policy.record(failure):
            raise failure.error
        self._policy.ask(failure)

    def run(self) -> None:
        """Reads every notification until all listeners end, calling their
        triggers, then closes them. Ctrl+C, or a failure that is raised, also
        closes them."""
        try:
            for _ in self:
                pass
        finally:
            self.close()

    def close(self) -> None:
        """Closes every listener. Safe to call more than once."""
        if not self._closed:
            self._closed = True
            self._raw.close()

    def __enter__(self) -> MultiNotificationIterator:
        return self

    def __exit__(self, *exc: object) -> bool:
        self.close()
        return False

    def __repr__(self) -> str:
        return (
            f"MultiNotificationIterator(listeners={self._policy.count}, "
            f"failures={len(self._policy.failures)})"
        )


class AsyncMultiNotificationIterator:
    """As ``MultiNotificationIterator``, for ``AsyncAvisoClient.listen_many``.

    Use ``async with`` and ``async for``, or ``await run()``. Function
    triggers may be ``async def``; they are awaited.
    """

    # reason: raw is the private native iterator; its items are typed in the
    # public stubs, not here.
    def __init__(
        self, raw: Any, functions: dict[str, list[_Function]], on_error: OnError, count: int
    ) -> None:
        self._raw = raw
        self._functions = functions
        self._policy = _Policy(on_error, count)
        self._closed = False
        self._stopped = False

    @property
    def errors(self) -> list[ListenFailure]:
        """Every failure so far, in order."""
        return list(self._policy.failures)

    def __aiter__(self) -> AsyncMultiNotificationIterator:
        return self

    async def __anext__(self) -> tuple[str, Any]:
        if self._stopped:
            raise StopAsyncIteration
        try:
            return await self._next()
        except asyncio.CancelledError:
            # Cancelling one read (a timeout around it, for example) leaves the
            # listeners open, as it does for listen().
            raise
        except BaseException:
            self._stopped = True
            await self.aclose()
            raise

    async def _next(self) -> tuple[str, Any]:
        while True:
            try:
                name, item = await self._raw.__anext__()
            except StopAsyncIteration:
                await self.aclose()
                self._policy.check_all_failed()
                raise
            if isinstance(item, BaseException):
                await self._fail(
                    ListenFailure(name, ListenFailureKind.LISTENER, _named(name, item))
                )
                continue
            try:
                await acall_functions(item, self._functions.get(name, []))
            except TriggerError as e:
                await self._fail(ListenFailure(name, ListenFailureKind.TRIGGER, _named(name, e)))
                continue
            return name, item

    async def _fail(self, failure: ListenFailure) -> None:
        if self._policy.record(failure):
            raise failure.error
        if callable(self._policy.on_error):
            result = self._policy.on_error(failure.listener, failure.error)
            if inspect.isawaitable(result):
                await result

    async def run(self) -> None:
        """Reads every notification until all listeners end, calling (and
        awaiting) their triggers, then closes them."""
        try:
            async for _ in self:
                pass
        finally:
            await self.aclose()

    async def aclose(self) -> None:
        """Closes every listener. Safe to call more than once."""
        if not self._closed:
            self._closed = True
            await self._raw.aclose()

    async def __aenter__(self) -> AsyncMultiNotificationIterator:
        return self

    async def __aexit__(self, *exc: object) -> bool:
        await self.aclose()
        return False

    def __repr__(self) -> str:
        return (
            f"AsyncMultiNotificationIterator(listeners={self._policy.count}, "
            f"failures={len(self._policy.failures)})"
        )
