# Architecture

```text
┌────────────────────────────────────────────────────────────────────────────────┐
│  Consumers                                                                     │
│  ┌──────────────────────┐         ┌──────────────────────────────────────┐     │
│  │ aviso-cli            │         │ aviso (Python package)               │     │
│  │ (Rust binary,        │         │   ┌────────────────────────────────┐ │     │
│  │  installed as        │         │   │ aviso-py (PyO3 binding crate;  │ │     │
│  │  `aviso`)            │         │   │  cdylib once bindings land)    │ │     │
│  └──────────┬───────────┘         │   └─────────────────┬──────────────┘ │     │
│             │                     └─────────────────────┼────────────────┘     │
│             └──────────────┬──────────────────────────┬─┘                      │
│                            ▼                          ▼                        │
│                     ┌────────────────────────────────────────┐                 │
│                     │  aviso  (core Rust library)            │                 │
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
