from __future__ import annotations

import asyncio

import aviso

POLYGON = "80,0,81,0,81,1,80,0"
EVENT_TYPE = "test_polygon"
PARALLELISM = 5


async def _publish(client: aviso.AsyncAvisoClient, seq: int) -> aviso.NotifyResponse:
    return await client.notify(
        event_type=EVENT_TYPE,
        identifier={"polygon": POLYGON, "date": "20260609", "time": f"{seq:04d}"},
        payload={"seq": seq},
    )


async def test_five_parallel_publishes_get_unique_request_ids_and_all_arrive_on_listen(
    base_url: str, producer_auth: aviso.Basic
) -> None:
    client = aviso.AsyncAvisoClient(base_url=base_url, auth=producer_auth)
    received: set[int] = set()
    async with client.listen(EVENT_TYPE, filter={"polygon": POLYGON}) as iterator:
        await asyncio.sleep(0.5)
        responses = await asyncio.gather(*(_publish(client, i) for i in range(1, PARALLELISM + 1)))

        request_ids = {r.request_id for r in responses}
        assert len(request_ids) == PARALLELISM, "every publish must get a unique request_id"

        async for notification in iterator:
            received.add(notification.payload["seq"])
            if len(received) >= PARALLELISM:
                break

    assert received == set(range(1, PARALLELISM + 1))
