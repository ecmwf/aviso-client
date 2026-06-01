"""Asynchronous listener: the async counterpart to basics/02_listen.py.

Use ``AsyncAvisoClient`` from inside any asyncio context (FastAPI,
aiohttp, a Jupyter cell with await, your own asyncio.run). The shape
mirrors the sync API; only the call style changes.

Expected output (one line per matching publish):

    seq=80 payload={'location': '...'}
    seq=81 payload=...
    seq=82 payload=...
    received 3 notifications; exiting
"""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pyaviso
from _common import require_env


async def main() -> None:
    client = pyaviso.AsyncAvisoClient(base_url=require_env(), auth=pyaviso.Env())
    count = 0
    async with client.listen("test_polygon", filter={"polygon": "0,0,1,0,1,1,0,0"}) as iterator:
        async for n in iterator:
            print(f"seq={n.sequence} payload={n.payload}")
            count += 1
            if count >= 3:
                break
    print(f"received {count} notifications; exiting")


if __name__ == "__main__":
    asyncio.run(main())
