# advanced

Patterns you reach for when the basics are not enough.

- **01_builder_pattern.py** -- the `WatchRequest.watch(...).with_filter(...).with_triggers([...])` alternative to the kwargs path on `client.listen()`. Use this when you want to build the request once (perhaps from configuration) and pass it to several listen calls or store it in a registry. Same scenario as `triggers/01_echo.py` for direct comparison.
- **02_replay_only.py** -- `mode="replay_only"` with `from_=<sequence>` for one-shot backfill scripts that exit at end-of-stream rather than keep listening forever. Self-contained: publishes a few notifications under a polygon unique to the script, then replays them.
