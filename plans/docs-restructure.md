# Docs restructure plan (for end users, not developers)

This document survives context compaction. A fresh session reading it
should be able to pick up the work without re-deriving anything.

## Status at time of save

- PR #16 (the Rust CLI) merged to `main` at `3a2005c9` on 2026-05-24.
- 79 commits landed. `feat/cli` branch is still present locally and on
  remote (cleanup not requested yet).
- Working tree clean before this file was added.
- All 43 Copilot PR review threads resolved across 9 rounds.
- Phase 6 (PyO3 bindings) is NOT yet started. `crates/aviso-py` is a
  placeholder `rlib`; `python/aviso/__init__.py` is a version stub only.
  `pyproject.toml` is wired with maturin so the cutover to a `cdylib`
  is a code-only change.

## What the user asked for (verbatim)

> next I want to restructure the documentation, right now it is not
> really for users, it is for developers how to integrate etc. An
> early restructure now makes sense to me, we should have docs for
> python api, python cli, rust cli and then for develeopers section,
> we can explain lib there, then bindings, then as an example how to
> create cli for c++ (all these can be done when the time comes, not
> now). It should be clear for the users and accurate, easy to follow,
> humanised text with less technical terms. Quick start for each, how
> to install, how to use etc. make a great plan, then ask oracle for
> a deep plan review, then show me the plan

## Process completed before save

1. Three parallel exploration agents run:
   - `bg_abccef90`: mapped current docs structure (24 .md files,
     ~3,500 lines, 0 Python pages, 5 dead-link placeholders, 2
     audience-conflated pages, 1 stub).
   - `bg_f2bbe71c`: mapped Python distribution state (placeholder
     `rlib`, version stub, maturin wired, no Python CLI planned).
   - `bg_3ccc9988`: best-practice research across 8 multi-language
     SDK docs (Polars, Ruff, Tokio, FastAPI, HTTPX, Maturin,
     pydantic-core, AWS SDK Rust).

2. Plan v1 drafted at `/tmp/docs-restructure-plan.md` (volatile;
   superseded).

3. Oracle plan review `bg_08b741b5` (1m33s) — verdict
   **PROCEED-AFTER-AMENDMENTS**. Oracle pushed back hard on:
   - Adding a "Python CLI" section for a binary that does not exist
     (reversal: use Option A, one CLI section serves everyone).
   - Pre-building a 10-page Python tree of "coming soon" stubs
     (placeholder fatigue — biggest underweighted risk).
   - 10 micro-pages for CLI (cut to 5-7 task-grouped pages).
   - `mdbook-tabs` third-party preprocessor (use stacked code blocks).
   - Phase A scope creep (defer recipes, bindings-guide,
     adding-a-language, error-ux to Phase C).
   - 3-4 day estimate (realistic is 5-7 days).
   - Keeping ADRs anywhere in mdBook (user already said may delete).

4. Plan v2 drafted at `/tmp/docs-restructure-plan-v2.md` (volatile;
   content reproduced below for durability).

5. User asked me to save context before compaction. This file.

## Final structure (v2)

```
docs/src/
├── SUMMARY.md
├── index.md                       Hero. Stacked Python | Rust | CLI examples.
│
├── getting-started/
│   ├── overview.md                What aviso is, three ways to use it, what to read next
│   ├── install.md                 Install matrix
│   ├── quickstart.md              First successful publish + listen, stacked code blocks
│   └── concepts.md                Teaser of the 5 concepts (each links to /concepts/)
│
├── cli/                           THE canonical CLI docs. Python + Rust users both link here.
│   ├── overview.md                What it does, when to use vs library
│   ├── install.md                 cargo install + prebuilt download
│   ├── quickstart.md              First 5-10 commands
│   ├── publish-and-listen.md      aviso notify + aviso listen (daily verbs)
│   ├── replay.md                  aviso replay
│   ├── operations.md              aviso schema + aviso admin + aviso config + completions
│   ├── configuration.md           Config file, env vars, precedence, TLS, signals, exit codes
│   └── troubleshooting.md         Expanded from 7-line stub: common operator issues
│
├── python/
│   └── status.md                  "Python bindings are not yet available. Use the CLI for now."
│                                  When Phase 6 ships, this page is replaced with the real tree.
│
├── triggers/                      Shared across audiences (YAML is common)
│   ├── overview.md                Six built-in kinds + when to use
│   ├── echo.md                    (existing, light polish)
│   ├── log.md
│   ├── command.md
│   ├── webhook.md
│   ├── teams.md
│   ├── post.md
│   └── template-engine.md
│
├── concepts/                      Short, canonical, every-audience-reads
│   ├── notifications.md           Wire shape, identifiers, payload, sequence
│   ├── streams.md                 SSE, reconnect, heartbeat
│   ├── resume-and-state.md        At-least-once, checkpoints, state file
│   ├── filters.md                 Identifier matching, required: true vs false
│   ├── auth-providers.md          The 5 providers + how to choose
│   └── glossary.md                Plain definitions (8 terms)
│
├── reference/
│   ├── cli-flags.md               clap-markdown generated, committed, CI freshness gate
│   ├── listener-yaml.md           Listener YAML schema reference
│   ├── state-file.md              state.json schema reference
│   └── rust-api.md                Pointer to docs.rs/aviso
│
└── developers/
    ├── overview.md                Who this section is for + repo map
    ├── architecture.md            Crate dependency graph; expanded
    ├── lib-guide.md               How to use the `aviso` Rust crate as a dependency
    ├── contributing.md            Tests, gates, MSRV, add-a-trigger walkthrough
    └── docs-style.md              Voice, banned phrases, runnable-code contract
```

Total: **31 markdown files** (down from 24 current + would-be ~50 in v1).

## Migration mapping

| Current path | New path | Notes |
|---|---|---|
| `welcome/introduction.md` | `index.md` (rewritten) | Hero with stacked code blocks |
| `welcome/installation.md` | `getting-started/install.md` + `cli/install.md` (split) | Audience-specific |
| `welcome/quick-start.md` | `getting-started/quickstart.md` (Rust tab) + `developers/lib-guide.md` | Current page is purely Rust |
| `welcome/key-concepts.md` | `getting-started/concepts.md` + `concepts/*` (extracted) | Front-door teaser + per-concept pages |
| `usage/notify.md` | `developers/lib-guide.md` | Rust-API-focused |
| `usage/schema.md` | `cli/operations.md` + `developers/lib-guide.md` | Split per audience |
| `usage/admin.md` | `cli/operations.md` + `developers/lib-guide.md` | Split |
| `usage/cli.md` (407 lines) | Slice into `cli/{quickstart,publish-and-listen,replay,operations,configuration,troubleshooting}.md` | 5-7 focused pages |
| `usage/auth.md` | `concepts/auth-providers.md` + `cli/configuration.md` + `developers/lib-guide.md` | Three-way split |
| `triggers/*` | `triggers/*` | Light polish |
| `watch/overview.md` (320 lines, conflated) | `concepts/streams.md` + `developers/lib-guide.md` | Split |
| `resume/overview.md` (102 lines, conflated) | `concepts/resume-and-state.md` + `developers/lib-guide.md` | Split |
| `resume/state-file.md` (323 lines, conflated) | `cli/configuration.md` + `reference/state-file.md` + `developers/architecture.md` | Three-way split |
| `troubleshooting/overview.md` (7 lines) | `cli/troubleshooting.md` (expanded to ~120 lines) | Grow |
| `internals/architecture.md` | `developers/architecture.md` (expanded with crate dep graph) | |
| `internals/contributing.md` (3 lines) | `developers/contributing.md` (expanded) | Inline gates + add-a-trigger walkthrough |
| `internals/decisions.md` (311 lines) | **Removed from `docs/src/` entirely** | Move to `plans/decisions.md` OR delete; not linked from any docs page |

## The 8 open questions (user has NOT answered yet)

When the user replies, they will pick on each:

1. **One CLI section serves both Python and Rust users?** My (oracle-aligned) lean: **YES**. No "Python CLI" section.

2. **One-page Python placeholder until Phase 6, not a pre-built tree?** Lean: **YES**. Single `python/status.md`.

3. **5-7 CLI pages (publish+listen grouped), not 10 micro-pages?** Lean: **YES**.

4. **ADR file: move to `plans/decisions.md` OR delete?** **DEFERRED to user**. User said earlier they'd "delete them at the end"; either option works.

5. **Stacked code blocks instead of `mdbook-tabs` preprocessor?** Lean: **YES**.

6. **Recipes (systemd/docker/k8s) deferred to Phase C, not in Phase A?** Lean: **YES**.

7. **Accept 5-7 day estimate, or push further?** Lean: **ACCEPT**.

8. **CLI reference committed file + CI freshness gate (not CI-generated)?** Lean: **YES**.

## Banned phrases in user-facing pages

Codified in `developers/docs-style.md` (to be created in Phase A):

- "the implementation lives at"
- "see ADR D4" (any ADR id)
- "per the locked design"
- "amendment" (in this PR context)
- "Q9" (any locked-design question id)
- "Phase 6", "Phase 7" (any phase number — user said no time-bound refs)
- "PR-G2" (any PR id)
- "stable-channel" (jargon)

Acceptable in `developers/` only: architectural prose, file paths, cargo crate names, ADR refs (if we keep them).

## Voice rules (in `developers/docs-style.md`)

- Lead with a verb operators say out loud ("Listen for new notifications", not "Use `AvisoClient::watch()` to construct a `NotificationStream`").
- First code block on every page is runnable. The only edit allowed is replacing `https://aviso.example` with the user's server URL.
- "You" not "the user". "aviso" (the tool) not "the aviso client suite".
- Stacked code blocks (no tabs): bolded language heading then fenced block. Example structure:
  ```markdown
  **Python (pip install aviso, when available):**
  ```python
  import aviso
  ```
  **Rust crate (`cargo add aviso`):**
  ```rust
  use aviso::AvisoClient;
  ```
  **CLI (`aviso` binary):**
  ```bash
  aviso notify event=mars,class=od
  ```
  ```

## Cross-cutting design decisions

1. One mdBook site, six sections (matches 6/8 best-in-class projects).
2. Stacked code blocks, not tabs (no third-party preprocessor).
3. Rust API reference on docs.rs (`reference/rust-api.md` is a one-page pointer).
4. Python API reference deferred until Phase 6 ships.
5. CLI reference: `clap-markdown` generated, committed to repo, CI freshness gate (PRs that change CLI surface without regenerating fail).
6. Humanised tone codified in `developers/docs-style.md`.
7. Tutorial / How-to / Reference three-layer arc per audience.
8. Concepts are short and canonical (80-150 lines each).
9. Glossary in `concepts/glossary.md` (8 plain definitions).
10. Future Python content fills `python/status.md` location, not pre-built files, until Phase 6.

## Link preservation (mdBook redirects)

Add to `book.toml`:

```toml
[output.html.redirect]
"welcome/quick-start.html" = "/getting-started/quickstart.html"
"welcome/introduction.html" = "/"
"usage/cli.html" = "/cli/quickstart.html"
"watch/overview.html" = "/concepts/streams.html"
"resume/state-file.html" = "/cli/configuration.html"
"internals/decisions.html" = "/plans/decisions.html" # if kept in plans/, otherwise omit (404 acceptable)
```

## Phasing

- **Phase A (this restructure)** — 5-7 working days. NOT YET STARTED.
  Scope:
  - All file moves + splits per migration table.
  - Hero `index.md` + universal `getting-started/quickstart.md` (stacked code blocks; Python tab is a placeholder pointing at `python/status.md`).
  - `python/status.md` as a single page ("CLI does what you need today").
  - `cli/*` 7-page split with `clap-markdown` reference (manual gen in A; CI gate as follow-up).
  - `concepts/*` 6 pages including glossary.
  - `developers/lib-guide.md` consolidating Rust quickstart + how-to.
  - Audience-conflated pages split per migration table.
  - Troubleshooting expanded to ~120 lines.
  - `docs-style.md` codifying voice rules.
  - mdBook redirects.
  - ADR file out of `docs/src/`.

- **Phase B (Phase 6 PyO3 bindings ship)** — 3-5 days additional.
  Scope: replace `python/status.md` with full Python tree
  (`overview.md`, `install.md`, `quickstart.md`, `publishing.md`,
  `listening.md`, `triggers.md`, `auth.md`, `examples/`); fill
  Python tab in stacked quickstart.

- **Phase C (post-Phase-6 polish)** — 2-3 days.
  Scope: GitHub Pages workflow, hosted pydoc/mkdocstrings,
  recipes (`cli/recipes/{systemd,docker,kubernetes}.md`),
  `developers/{bindings-guide,adding-a-language}.md`.

## Definition of done (Phase A)

- All 24 current files moved or split per migration table.
- `SUMMARY.md` rewritten.
- All 5 placeholder dead links resolved.
- `mdbook build docs` and `mdbook test docs` clean.
- New operator finds "How to install and run `aviso listen`?" in <60s with no Rust knowledge.
- New Python user lands on `python/status.md` within one click and sees clear next-step guidance.
- Audience-conflated pages split.
- Troubleshooting ≥ 120 lines, not 7.
- Banned-phrases rules respected in user-facing pages (manual check for Phase A; CI gate as follow-up).
- `reference/cli-flags.md` generated by `clap-markdown`.
- mdBook redirects for 5 moved paths.
- ADR file gone from `docs/src/`.

## Risks (ranked)

1. **Placeholder fatigue (highest).** Even one `python/status.md` risks signaling "this product isn't ready". Mitigation: write the page as "CLI does what you need today" not "Python is coming".
2. **`cli.md` slicing.** 407-line monolith with internal cross-links. Mitigation: every heading in monolith maps to heading in new page; mdBook fails build on broken links.
3. **`clap-markdown` freshness gate is new CI work.** Mitigation: manual generation in Phase A; CI gate as follow-up.
4. **"Humanised tone" rule is subjective.** Mitigation: `docs-style.md` pins rules concretely.

## What NOT to do (anti-patterns oracle warned about)

- Do not add an "Python CLI" section. There is no Python CLI binary.
- Do not pre-build the 10-page Python tree before PyO3 ships. Use one status page.
- Do not split CLI into 10 micro-pages. Group daily verbs.
- Do not depend on `mdbook-tabs` or any third-party preprocessor.
- Do not put recipes (systemd/docker/k8s) or bindings-guide or adding-a-language in Phase A. Defer to Phase C.
- Do not link ADRs from any user-facing page.
- Do not name phase numbers ("Phase 6"), PR ids ("PR-G2"), or ADR ids ("D4") in user-facing pages.
- Do not promise the Python timeline in `python/status.md`. Say "the CLI does what you need today; Python bindings are a separate work item".

## Inputs that informed this plan (durable references)

- `plans/v0.3.md` — the v0.3 plan with Phase 6 scope (PyO3 bindings).
- Current `docs/src/SUMMARY.md` — has 5 placeholder dead links.
- Current `docs/src/welcome/quick-start.md` — Rust-only, first impression is Rust deps in Cargo.toml.
- Current `crates/aviso-py/{Cargo.toml,src/lib.rs}` — placeholder rlib.
- Current `python/aviso/__init__.py` — version stub only.
- Current `pyproject.toml` — fully wired with maturin, `aviso._native` module name.
- Current `docs/book.toml` — uses mdBook with `rust` theme.

## Resume instructions for a fresh session

1. Read this file completely.
2. Check `git log -1 --format='%H %s'` — should be at or after `3a2005c9` (PR #16 merge).
3. Check `git status` — should be clean.
4. Ask the user the 8 open questions above (if not already answered).
5. Once answered, start Phase A. Begin by creating the new directory structure under `docs/src/` and moving the lowest-risk content first (the `triggers/*` directory needs almost no change). Then tackle the cli.md slicing, which is the riskiest single piece of work.
6. Run gates after each section: `cargo fmt --all -- --check`, `mdbook build docs`, `mdbook test docs`.
7. Banned-phrases sweep on the final diff: `git diff --unified=0 | grep -E '^\+' | grep -iE 'ADR |Phase [0-9]|amendment|PR-[A-Z][a-z]|Q[0-9]+\s+|locked design'` should return nothing for files outside `developers/`.
8. Em-dash sweep: same diff, count `\u2014`, must be 0.
9. After Phase A is implemented, ask oracle for a code review before committing (pattern used throughout this PR session).
10. Commit on a feature branch, NOT directly to main. Open a PR.

## Oracle review trail

- `bg_08b741b5` — plan v2 review (1m33s, PROCEED-AFTER-AMENDMENTS).
  Session id `ses_1a609d47dffeVLUd9Ds3yEOlzr`. The complete review
  output is in the conversation history; do not re-run.
- `bg_abccef90` — current docs inventory (1m27s).
  Session id `ses_1a60f166effeQUZQOge3G7N8lx`.
- `bg_f2bbe71c` — Python distribution state (1m21s).
  Session id `ses_1a60ef341ffe3P1xgQl2yJIACF`.
- `bg_3ccc9988` — best-practice research across 8 projects (51s).
  Session id `ses_1a60eb28bffeuqdK4HaYbPnPf6`.
