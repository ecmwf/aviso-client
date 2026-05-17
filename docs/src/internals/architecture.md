# Architecture

> Phase 0: high-level only.

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│  Consumers                                                                     │
│  ┌──────────────────────┐         ┌──────────────────────────────────────┐     │
│  │ aviso-client-cli     │         │ aviso_client (Python package)        │     │
│  │ (Rust binary)        │         │   ┌────────────────────────────────┐ │     │
│  │                      │         │   │ aviso-client-py (cdylib via    │ │     │
│  │                      │         │   │  PyO3 — Phase 5)               │ │     │
│  └──────────┬───────────┘         │   └─────────────────┬──────────────┘ │     │
│             │                     └─────────────────────┼────────────────┘     │
│             └──────────────┬──────────────────────────┬─┘                      │
│                            ▼                          ▼                        │
│                     ┌────────────────────────────────────────┐                 │
│                     │  aviso-client (core Rust library)      │                 │
│                     │  • HTTP via reqwest + rustls           │                 │
│                     │  • SSE parser via sse-core             │                 │
│                     │  • Reconnect supervisor + state        │                 │
│                     │  • AuthProvider trait                  │                 │
│                     │  • Trigger dispatcher (echo, log)      │                 │
│                     │  • StateStore (memory, JSON-file)      │                 │
│                     └────────────────────────────────────────┘                 │
└────────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
                            HTTP + SSE
                                    │
                                    ▼
                          aviso-server (out-of-tree)
```

The CLI and the Python extension are *peer consumers* of the core library and never depend on each other. The core library never depends on PyO3, on actix, or on any CLI machinery.

See [Architectural decisions](./decisions.md) for the rationale behind every load-bearing choice.
