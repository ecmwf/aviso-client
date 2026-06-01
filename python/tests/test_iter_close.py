"""Iterator close semantics.

The sync `NotificationIterator.close()` and async
`AsyncNotificationIterator.aclose()` cooperate with the supervisor's
final checkpoint flush when `flush_cursor_on_exit=True` is set on the
client. After close, both iterators behave as exhausted (StopIteration /
StopAsyncIteration) rather than raising RuntimeError.
"""

from __future__ import annotations

import asyncio

import pyaviso
import pytest


def test_sync_close_after_open_is_idempotent() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"})
    iterator.close()
    iterator.close()


def test_sync_close_then_next_raises_stop_iteration() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"})
    iterator.close()
    with pytest.raises(StopIteration):
        next(iterator)


def test_async_aclose_then_anext_raises_stop_async_iteration() -> None:
    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"})

    async def drive() -> None:
        await iterator.aclose()
        with pytest.raises(StopAsyncIteration):
            await iterator.__anext__()

    asyncio.run(drive())


def test_async_aclose_is_idempotent() -> None:
    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"})

    async def drive() -> None:
        await iterator.aclose()
        await iterator.aclose()

    asyncio.run(drive())


def test_sync_iterator_is_iterable() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"})
    assert iter(iterator) is iterator
    iterator.close()


def test_async_iterator_is_async_iterable() -> None:
    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")
    iterator = client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"})
    assert iterator.__aiter__() is iterator

    async def cleanup() -> None:
        await iterator.aclose()

    asyncio.run(cleanup())


def test_sync_iterator_works_as_context_manager() -> None:
    client = pyaviso.AvisoClient(base_url="http://127.0.0.1:1")
    with client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}) as iterator:
        assert iter(iterator) is iterator
    with pytest.raises(StopIteration):
        next(iterator)


def test_async_iterator_works_as_async_context_manager() -> None:
    client = pyaviso.AsyncAvisoClient(base_url="http://127.0.0.1:1")

    async def drive() -> None:
        async with client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}) as iterator:
            assert iterator.__aiter__() is iterator
        with pytest.raises(StopAsyncIteration):
            await iterator.__anext__()

    asyncio.run(drive())
