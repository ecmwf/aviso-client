# Standing constraints

Product and process rules that bound every decision. Do not override them without the user saying so. Quoted phrases are the user's own words.

- **Never merge without explicit approval.** "never ever merge without asking." The PR flow always stops at the merge gate, even with green CI and a clean review.
- **No speculative versioning or over-engineering.** "NEVER DO THINGS LIKE V2 etc. ... this is not used anywhere ... dont over engineer."
- **No backwards-compatibility burden.** The package is unreleased; breaking changes are fine when they improve the design.
- **No SQLite or database backends.** "I dont want sqlite, it is overengineering." State is a JSON file (`JsonFileStore`) plus an in-memory store for tests.
- **One config directory.** "lets make it ~/.config/aviso for the location of the config and the state." `config.yaml` and `state.json` both live there.
- **No managed services.** "no aws etc. ... we won't use managed services." Triggers use operator-configured generic protocols only: SMTP for email, HTTP for webhook. The user supplies the endpoint and credentials; aviso ships the generic client. This rules out AWS SES, SendGrid, Mailgun, and any cloud-vendor SDK dependency.
- **Binding-friendly core.** "I want this to be run in both python and c++ easily." The Rust core stays neutral: `AvisoClient` is `Send + Sync + Clone`, one mpsc channel per watch, no closure-heavy public surfaces unless necessary.
- **Commits, docs, and PRs describe substance, not process.** Phase names, internal PR ids, tool or reviewer names, and review-round counters appear only under `plans/`, never in code, docs, commit messages, or PR bodies. See [`AGENTS.md`](../AGENTS.md) for the full rule.
- **Plan-only commits may push straight to `main`;** source-code changes go through a PR.
- **Never add `Co-authored-by:` trailers.**
