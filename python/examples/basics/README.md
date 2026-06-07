# basics

The three calls every user touches first.

- **01_publish.py** -- construct a client, publish one notification, print the server's `request_id` and `processed_at`.
- **02_listen.py** -- iterate a watch stream and print each notification. Stops after 3 so the example completes; remove `break_after` for a real long-running listener.
- **03_schema_discovery.py** -- list the event types the server has configured and inspect one schema's identifier fields.
- **04_publish_many.py** -- publish several notifications in one concurrent `notify_many` call and print each per-item result.
