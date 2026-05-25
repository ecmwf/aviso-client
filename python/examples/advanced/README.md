# advanced

Patterns you reach for when the basics are not enough.

- **01_builder_pattern.py** -- the `WatchRequest.watch(...).with_filter(...).with_triggers([...])` alternative to the kwargs path on `client.listen()`. Use this when you want to build the request once (perhaps from configuration) and pass it to several listen calls or store it in a registry. Same scenario as `triggers/01_echo.py` for direct comparison.
- **02_replay_only.py** -- `mode="replay_only"` with `from_=<sequence>` for one-shot backfill scripts that exit at end-of-stream rather than keep listening forever.
- **03_webhook_with_local_server.py** -- the runnable counterpart to `triggers/05_webhook.py`: spins up an in-process `http.server.HTTPServer` on a random port, points the webhook trigger at it, asserts a delivery before exit.
