# (C) Copyright 2024- ECMWF and individual contributors.
#
# This software is licensed under the terms of the Apache Licence Version 2.0
# which can be obtained at http://www.apache.org/licenses/LICENSE-2.0.
# In applying this licence, ECMWF does not waive the privileges and immunities
# granted to it by virtue of its status as an intergovernmental organisation nor
# does it submit to any jurisdiction.

"""Drain two test_polygon streams concurrently with asyncio.gather.

Two separate listen calls, two separate async iterators, both running
under a single asyncio loop on the client's shared HTTP connections. Each
drain task tags its output with the polygon it matched so you can see
the interleaving.

Expected output (the two tags interleave depending on publish timing):

    [square-a] seq=...
    [square-b] seq=...
    [square-a] seq=...
    [square-b] seq=...
    done
"""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pyaviso
from _common import require_env


async def drain(client: pyaviso.AsyncAvisoClient, tag: str, polygon: str) -> None:
    count = 0
    async with client.listen("test_polygon", filter={"polygon": polygon}) as iterator:
        async for n in iterator:
            print(f"[{tag}] seq={n.sequence}")
            count += 1
            if count >= 2:
                return


async def main() -> None:
    client = pyaviso.AsyncAvisoClient(base_url=require_env(), auth=pyaviso.Env())
    await asyncio.gather(
        drain(client, "square-a", "0,0,1,0,1,1,0,0"),
        drain(client, "square-b", "2,2,3,2,3,3,2,2"),
    )
    print("done")


if __name__ == "__main__":
    asyncio.run(main())
