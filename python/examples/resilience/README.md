# resilience

The two patterns that make a listener survive things going wrong.

- **01_resume_with_state_store.py** -- attach a `JsonFileStore` so the supervisor commits the cursor on each notification. Restarts pick up after the last committed sequence instead of replaying from the live edge. `flush_cursor_on_exit=True` + the iterator's `with` form commits the last notification on clean shutdown.
- **02_error_handling.py** -- deliberately publishes an invalid notification to trigger `HttpError`, then prints the structured fields (`status`, `request_id`, `body`). Demonstrates the `aviso.AvisoError` hierarchy for fine-grained vs. catch-all dispatch.
