from __future__ import annotations

import asyncio

import pyaviso
from _helpers import receive_within_async

POLYGON_A = "50,0,51,0,51,1,50,0"
POLYGON_B = "60,0,61,0,61,1,60,0"
EVENT_TYPE = "test_polygon"


async def _consume(client: pyaviso.AsyncAvisoClient, polygon: str, count: int) -> list[int]:
    received: list[int] = []
    async with client.listen(EVENT_TYPE, filter={"polygon": polygon}) as iterator:
        for _ in range(count):
            notification = await receive_within_async(iterator, timeout=10)
            received.append(notification.payload["seq"])
    return received


async def _publish_one(
    client: pyaviso.AsyncAvisoClient, polygon: str, date: str, time_str: str, seq: int
) -> None:
    await client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": polygon, "date": date, "time": time_str},
        payload={"seq": seq, "polygon": polygon},
    )


async def test_two_async_listeners_drain_disjoint_polygons(
    base_url: str, producer_auth: pyaviso.Basic
) -> None:
    client = pyaviso.AsyncAvisoClient(base_url=base_url, auth=producer_auth)

    async def feed_a() -> None:
        await asyncio.sleep(0.7)
        for i in range(1, 4):
            await _publish_one(client, POLYGON_A, "20260607", f"{i:04d}", i)

    async def feed_b() -> None:
        await asyncio.sleep(0.7)
        for i in range(10, 13):
            await _publish_one(client, POLYGON_B, "20260607", f"{i:04d}", i)

    consumer_a = asyncio.create_task(_consume(client, POLYGON_A, 3))
    consumer_b = asyncio.create_task(_consume(client, POLYGON_B, 3))
    await asyncio.sleep(0.5)
    feeders = asyncio.gather(feed_a(), feed_b())

    a_results, b_results = await asyncio.gather(consumer_a, consumer_b)
    await feeders

    assert sorted(a_results) == [1, 2, 3]
    assert sorted(b_results) == [10, 11, 12]
