# Quickstart

Three recipes. Pick the one closest to your workflow.

## 1. Publish one notification

```python
import aviso

client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    auth=aviso.Bearer("opaque-jwt"),
)

response = client.notify(
    event_type="mars",
    identifier={"class": "od", "stream": "oper", "date": "20260601"},
    payload={"location": "s3://bucket/path/to/data"},
)

print(response.status, response.request_id)
```

## 2. Listen for notifications (sync)

```python
import aviso

client = aviso.AvisoClient(base_url="https://aviso.example.org")

for notification in client.listen("mars", filter={"class": "od"}):
    print(notification.sequence, notification.identifier, notification.payload)
```

Press `Ctrl+C` to stop. The iterator raises `KeyboardInterrupt` within ~100 ms of the signal.

## 3. Listen for notifications (async)

```python
import asyncio
import aviso

async def main() -> None:
    client = aviso.AsyncAvisoClient(base_url="https://aviso.example.org")
    async for notification in client.listen("mars"):
        print(notification.sequence, notification.payload)

asyncio.run(main())
```

## Resume across restarts

```python
import aviso

client = aviso.AvisoClient(
    base_url="https://aviso.example.org",
    state_store=aviso.JsonFileStore("~/.config/aviso/state.json"),
)

for notification in client.listen("mars"):
    print(notification.sequence)
```

The first run reads from the live edge. Subsequent runs pick up from the last committed sequence. The file is locked across cooperating processes on local filesystems.
