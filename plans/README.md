# Plans

This directory carries the project's planning documents. Plans are the **only** place in the repo where phase numbers and roadmap dates appear (per the rule in [`AGENTS.md`](../AGENTS.md#time-bound-references)).

Plans survive context compaction and fresh sessions: an agent or contributor coming in cold can read the current plan here and pick up without re-deriving everything from scratch.

## Current plan

- [`v0.3.md`](./v0.3.md): overall plan for the aviso-client suite (server facts that drive design, architecture summary, roadmap, follow-ups, open questions).

## Conventions

- One plan revision per file (`vMAJOR.MINOR.md`). When the plan changes meaningfully, write a new file and update this README.
- Plans are descriptive, not prescriptive at the line level. Architectural decisions (the *why* of each design choice) live in [`decisions.md`](./decisions.md) and are referenced from plans by their stable ADR id (D1, D2, …).
- Plans may name phases. Code, user-facing docs, and configuration files may not.
