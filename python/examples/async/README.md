# async

The two patterns that justify using `AsyncAvisoClient` instead of the default `AvisoClient`.

- **01_basic.py** -- the async counterpart of `basics/02_listen.py`: `await` + `async for` + the iterator's `async with` form. Use this shape when you are inside an existing event loop (FastAPI, aiohttp, a Jupyter cell).
- **02_multiplex.py** -- two listeners draining two `test_polygon` shapes concurrently with `asyncio.gather`. The case where async earns its keep: doing in one process what would otherwise require two threads.
