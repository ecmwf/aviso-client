# Plans

Where the project is going and where it has been. Keep it short: the detail belongs in the linked PRs, in the code, and in the ADRs.

- [`roadmap.md`](./roadmap.md): what is next.
- [`progress.md`](./progress.md): what has shipped, newest first.
- [`decisions.md`](./decisions.md): the architectural decision log (ADRs, stable `D1`..`Dn` references) and the server facts that drive them.
- [`constraints.md`](./constraints.md): standing product and process rules that bound every decision.

## Conventions

- This folder is the only place that may name internal process detail: phase names, review rounds, tool or reviewer names. Code, docs, commit messages, and PR bodies must not.
- `progress.md` gets one line per merged PR. If an entry needs more, link the PR and stop.
- `roadmap.md` stays forward-looking; move an item to `progress.md` when it ships.
- ADRs in `decisions.md` are appended, and amended in place with a dated note; their ids are stable cross-references.
