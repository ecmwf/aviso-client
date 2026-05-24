# Documentation style

The conventions documentation pages in this book follow. If you are writing or reviewing a page, this is the contract.

## Voice

- Lead with what the user is trying to do, not with how the code is laid out. "Listen for new notifications" rather than "Use `AvisoClient::watch()` to construct a `NotificationStream`".
- Address the reader directly. "You" instead of "the user". "aviso" (the tool or the library) instead of "the aviso client suite" or "the aviso-client project".
- Keep sentences short. Two short ones almost always read better than one long one.
- Avoid jargon when a plain word would do. "Resume from where you left off" rather than "restore checkpointed state".

## Runnable examples

- The first code block on each user-facing page should be runnable as written. The only edit the reader should need is replacing `https://aviso.example` with their server URL.
- Long examples are fine, but break them into pieces with prose between them. Walls of code without explanation are hard to navigate.
- Use real values where possible. `mars` and `class=od` rather than `<event-type>` and `<key>=<value>`.

## Headings

- Title case for the page title (`# Resume and state`).
- Sentence case for sub-headings (`## When the cursor advances`).
- Each H2 should answer a question the reader will actually ask. "When the cursor advances" not "Cursor advancement logic".

## Code blocks

- Always set the language: ` ```bash `, ` ```yaml `, ` ```rust,ignore `.
- Rust examples in user-facing pages use `rust,ignore` so doctests do not try to compile them. They are illustrative, not part of the test surface.
- For commands, prefer the multi-line form with `\` continuations only when the line is too long to read in one go.

## Tables

Use them when a comparison is the point. Not for prose that happens to have two columns.

```markdown
| You want to | Use |
|---|---|
| Print notifications to your terminal | `echo` |
| Append to a file | `log` |
```

## Cross-references

- Same book: relative path with the explicit `.md` (mdBook rewrites it). For example, a link from a `cli/` page to its quickstart sibling reads `[Quickstart](./quickstart.md)`.
- External: a full URL. `<https://docs.rs/aviso>`.
- Anchor to a section in the same page: `[name](#section)`.

## Things to avoid in user-facing pages

These belong in `developers/` or in `plans/`, not in pages people land on while learning aviso.

- References to planning-only rationale. Keep user-facing pages focused on what exists now and how to use it.
- Mentions of roadmap phases or planned-but-not-shipped features that imply the product is not yet ready.
- "The implementation lives at `<crate path>`". Users do not need to know.
- Crate-internal names like `TriggerKindLabel`, `ResumeKey`, `WatchEvent`. Mention them only in `developers/` and in the API reference.
- Stable event-name strings (`event.name = "client.resume.applied"`). Those are operator log diagnostics, not part of the conceptual model.

## Punctuation

- Hyphens (`-`) yes. Em dash characters no. They look alike to casual readers, and a hyphen reads cleanly in a terminal.
- One space after a period.
- No Oxford comma policy; do what reads best for the sentence.

## Length

A page is too long when a reader has to scroll to find the second example. Two screens of typical text is the upper end; one is better. If a page is growing past that, the right move is usually to split it.

## When in doubt

Write the explanation you would have wanted when you were first learning aviso. That tends to be shorter, plainer, and more useful than the explanation you would write for the version of yourself who already knows.
