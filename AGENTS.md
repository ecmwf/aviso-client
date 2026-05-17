## Workspace overview
- Public APIs: prefer changes via small, reviewed steps.
- Do not consider backwards compatibility when changing the implementation, this is an unreleased package
- CRITICAL: NEVER suppress warnings, lint errors, or test failures with annotations (#[allow(...)], noqa, @SuppressWarnings, etc.) unless the suppression itself is the correct semantic choice (e.g., #[allow(clippy::too_many_arguments)] on a function that genuinely needs many parameters). If a lint fires, fix the underlying code. If a test fails, fix the bug. If a warning appears, resolve the root cause. Quick workarounds that hide problems are strictly prohibited.
- CRITICAL: Always prefer proper solutions over quick fixes. When facing a problem, invest the effort to understand the root cause and fix it correctly rather than applying a workaround. Specifically:
- Do NOT skip platforms or configurations to avoid fixing a build failure.
- Do NOT add conditional compilation or feature gates to hide broken code.
- Do NOT remove or weaken tests to make CI pass.
- Do NOT add TODO/FIXME/HACK comments as a substitute for doing the work now.
- If a proper fix requires changing multiple files or modules, do it.
- If a proper fix requires understanding unfamiliar code, read it first.
- If you are unsure whether a fix is proper, ask before proceeding.
- IMPORTANT: when you build code and new features:
  - ALWAYS document those features in docs/
- IMPORTANT:
  - when you commit your work, make sure it passes all checks, tests and lints

## Planning rules
- Always propose an implementation plan first.
- Wait for approval before applying any code changes.
- Keep each change scoped to one concern (bugfix, feature, refactor, or tests), not all at once.
- When the user asks for a "second pass", "third pass", or says "usuals", treat it as shorthand for:
    - simplification opportunities,
    - naming/comment/doc quality review,
    - edge-case/logical regression scan,
    - and running required formatter/lint/tests.

## Crate boundaries
- Before extracting a new crate: ensure behavior is covered by tests; propose the new crate API surface and dependency direction.
- Prefer keeping dependency arrows one-way; avoid cycles.

## Test organization goals
- Unit tests stay close to code when they exercise internals.
- Integration tests in `/tests` cover public behavior across crate boundaries.
- Add shared integration helpers under `tests/common/` (minimal; no hidden magic).

## Comment style
- Do not use marker-style comments such as `NEW`, `AI`, `temporary`, or `quick fix`.
- Comments must be neutral and explain intent/invariants, not authorship or edit history.
- For parser/format-sensitive code, include at least one valid and one invalid example in comments.

## Time-bound references
- Phase numbers (`Phase 0`, `Phase 5+`, …), roadmap dates, and "lands in Phase N" remarks MUST NOT appear in code, docstrings, configuration files, commit messages of merged work, or user-facing docs. They belong to `plans/` only.
- The repo describes what currently is, not what phase produced it. Phase references in code and docs become stale artifacts the moment a phase ships and silently mislead readers who arrive months later.
- If you find yourself writing "Phase N scaffold" or "comes in Phase M" anywhere outside `plans/`, the correct action is one of:
  1. describe the current state in terms of what exists today,
  2. mark the entry as a draft chapter (or delete the page) until it has real content, or
  3. move the reference into `plans/`.
- The same rule applies to TODO-style status banners such as `> Status: Phase 0 — placeholder.` and to scaffold/preview markers in module docs and config comments.

## Commit conventions
- Use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/). Every commit subject is `<type>(<scope>): <description>` — lowercase, imperative mood, ≤72 chars, no trailing period.
- Recognised types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `meta`, `style`, `revert`. Pick the narrowest one that fits; do not invent new types without a reason in the body.
- Scope is optional but encouraged. Use the affected crate, module, or area (`fix(cargo): …`, `docs(plans): …`, `ci(deny): …`, `test(e2e): …`).
- Breaking changes append `!` after the type/scope and explain in the body: `feat(api)!: rename AvisoClient::watch`. Add a `BREAKING CHANGE:` footer when downstream callers must act.
- **One concern per commit.** A commit is a bugfix, a feature, a refactor, a test pass, or a doc update — not a mix. If the subject would naturally use "and", split.
- **Sensibly sized.** Aim for the smallest commit that still leaves the tree green. A 5-file commit that fixes one thing is better than a 40-file commit that does ten. Bootstrap-scale work splits across multiple commits — layout, build config, docs, CI, harness — each one buildable on its own.
- **Every commit on `main` builds and passes all checks** — the entire acceptance set (`cargo fmt --check`, `cargo clippy --locked -D warnings`, `cargo test --locked`, `mdbook build`, `mdbook test`, `cargo deny check`). Don't merge a series where intermediate commits are broken.
- Commit body explains the *why*. If the change is non-obvious, write prose, wrapped at ~72 chars. Cite issues in a trailer (`Refs #42`, `Closes #42`); record breaking-change notes in a `BREAKING CHANGE:` trailer.
- Never amend or force-push branches that others may have pulled. On private branches, amend freely.
- "Pass 1", "Pass 2", "Phase 0" and similar process-stage names are NOT commit types or scopes (see also the Time-bound references rule); use `meta`, `refactor`, or `chore` with a real scope instead.

# Rust rules
## Style

- **`cargo fmt` and `cargo clippy -- -D warnings`** must be clean. CI fails otherwise.
- **`#![deny(unsafe_op_in_unsafe_fn)]`** at crate root for any crate using `unsafe`.
- **`#![warn(missing_docs)]`** for libraries.
- **One module per concern.** Files >500 lines get split.
- **No `pub use` re-exports across more than one layer** without a clear reason.
- **Tests live next to code** for unit tests (`#[cfg(test)] mod tests`), in `tests/` for integration.
- **NEVER `println!`/`eprintln!` in library code.** Use `tracing` (or pass a writer in).

## Dependencies

- **Pin minor versions** (`tokio = "1.45"` not `"1"`). Lockfile committed.
- **Justify every dep** in the PR description if it's the first of its kind.
- **Default features off** for heavy crates: `tokio = { version = "1.40", default-features = false, features = ["..."] }`.

## Errors

- **NEVER `unwrap()` or `expect()` in non-test code.** Test code (`#[cfg(test)]`, `tests/`, doctests) may use them. The only allowed exception in `main` is a documented "fatal init" line at startup — and even there prefer `.context(...)?`.
- **NEVER swallow errors.** No `let _ = result;`, no `if let Err(_) = result {}` without action. If you genuinely don't care, write `.ok();` and add a `// reason: ...` comment.
- **MUST use `?` for propagation.** No `match result { Err(e) => return Err(e), Ok(v) => v }`.
- **Libraries: typed errors with `thiserror`.** No `Box<dyn Error>` in public library APIs. No `anyhow` in library crates.
- **Binaries: `anyhow::Result` is fine.** Add `.context("doing X")` at every layer transition so failure messages tell a story.
- **NEVER `panic!()` in library code.** A library that panics on user input is a library with a bug. Document any unavoidable panic in the function's `# Panics` doc section.
- **Use `Result<T, !>` (or `Infallible`) when a function cannot fail** but its trait signature requires `Result`.

## Ownership and borrowing

- **NEVER `clone()` to silence the borrow checker.** Fix the lifetime, restructure ownership, or accept `&T`. A `clone()` is a deliberate decision with a comment, not a fix.
- **Prefer `&str` over `String`, `&[T]` over `Vec<T>`** in function parameters unless ownership is required.
- **NEVER `Arc<Mutex<T>>` reflexively.** Single-threaded first; `Mutex` only when you've proven contention is unavoidable; `Arc` only when shared ownership is real.
- **Prefer `Cow<'_, str>`** when a function sometimes needs to allocate and sometimes doesn't.
- **Move semantics over references** when the value is consumed. Don't borrow what you'll drop anyway.

## Unsafe

- **Every `unsafe` block has a `// SAFETY:` comment** naming the invariant the caller relies on. No exceptions.
- **`unsafe fn` is a public contract.** Document preconditions in `# Safety`. Internal-only unsafety should be `unsafe { ... }` blocks inside safe functions.
- **MIRI must pass** for any crate touching raw pointers, `transmute`, or FFI.
- **NEVER `mem::transmute` between types you control.** Use `From`/`TryFrom` or rethink the layout.

## Types and APIs

- **Make invalid states unrepresentable.** Use enums for state machines, newtypes for domain values (`UserId(u64)` not `u64`).
- **`#[must_use]`** on builders, result-like types, and any function whose return must not be silently dropped.
- **NEVER `as` for narrowing casts** (`u64 as u32`). Use `u32::try_from(x)?`.
- **Public items have `///` docs** describing intent, not implementation. Examples for non-trivial APIs.
- **`#[non_exhaustive]`** on public enums and structs that may grow.
- **Prefer `impl Trait` in argument position**, concrete types in return position (or `impl Trait` only when the concrete type is genuinely an implementation detail).

# Python Rules
## Toolchain (Astral stack)

Default tools, all from [Astral](https://astral.sh). Pick alternatives only with a justified reason in the PR.

| Concern        | Tool       | Notes |
|----------------|------------|-------|
| Package mgmt   | `uv`       | env, install, lock, run; replaces pip/pip-tools/poetry |
| Lint           | `ruff check`  | replaces flake8, isort, pyupgrade, pylint subset |
| Format         | `ruff format` | replaces black |
| Type check     | `ty check`    | replaces mypy/pyright; fast, written in Rust |
| Python version | `.python-version` (managed by `uv`) | one source of truth |

## Typing

- **MUST use type hints on every public function and method.** Internal helpers should too unless trivial.
- **`ty check` must pass in strict mode.** CI fails otherwise. Treat `ty`'s warnings as errors in CI.
- **NEVER `Any` without a comment** explaining why no narrower type works.
- **`from __future__ import annotations`** at the top of every module.
- **Prefer `Protocol` and `TypedDict`** over `dict[str, Any]` and ad-hoc duck typing.
- **`Optional[X]` only when `None` is a real semantic state** ("absent" vs "empty"). Otherwise raise.

## Errors

- **NEVER bare `except:` or `except Exception:`** without re-raise. Catch the specific exception class.
- **NEVER swallow exceptions silently.** If you intentionally ignore one, log it at `debug` and add a comment.
- **Raise typed exceptions**, not `RuntimeError("everything's broken")`. Define a small hierarchy per package.
- **NEVER catch and re-raise as a different type** without `raise NewError(...) from original`.
- **`assert` is debug-only.** Don't use it for input validation; it's stripped under `python -O`.

## IO and resources

- **MUST use `with` blocks** for any resource with `close()` (`open`, sockets, db connections, locks).
- **`pathlib.Path`, never `os.path.join`** for new code.
- **NEVER `print()` in libraries or long-running scripts.** Use `logging` or `structlog`.
- **Logging config at the entry point only.** Libraries get a `logger = logging.getLogger(__name__)` and never `basicConfig`.
- **Read text with explicit encoding**: `open(path, encoding="utf-8")`. Defaults differ across platforms.

## Data shapes

- **Prefer `@dataclass(frozen=True, slots=True)`** for records. Use `pydantic` only when parsing untrusted input.
- **NEVER mutable default arguments.** `def f(x: list[int] = [])` is a bug. Use `None` and assign inside.
- **`Enum` (or `StrEnum` in 3.11+) for fixed string sets**, not bare strings.
- **f-strings only.** No `.format()`, no `%`.
- **`match` statements over long `if/elif` chains** when the dispatch is structural.

## CLIs

- **Use `typer` (default) or `click`.** Argparse only if you have a strong reason.
- **Every CLI has `--version`, `--help`, and exits non-zero on error** (typer/click do this for free if you let them).
- **Reserve exit codes**: `0` ok, `1` runtime error, `2` usage. Use codes >=64 for app-specific.
- **Provide `--dry-run`** for any command that writes/deletes/sends.
- **`if __name__ == "__main__":` calls a `main()` function**, not inline code.

## Async

- **NEVER mix sync and async carelessly.** `asyncio.run()` is the entry point — once.
- **Use `asyncio.TaskGroup`** (3.11+) for structured concurrency. Bare `asyncio.create_task` without an `await` later is a leak.
- **NEVER `time.sleep` in async code.** Use `asyncio.sleep`.
- **Cancellation: re-raise `asyncio.CancelledError`**, don't swallow. Clean up in `finally`.
- **Timeouts via `asyncio.timeout()`** (3.11+), not bespoke wall-clock checks.

## Packaging and tooling

- **`pyproject.toml` only.** No `setup.py`, no `requirements.txt` for new projects (committed lockfile via `uv` or `poetry` is fine).
- **`uv` is the default** package manager for new projects. Lockfile committed.
- **Pin a Python version** (`.python-version` file) so contributors and CI agree.
- **Format with `ruff format`, lint with `ruff check`** — both clean in CI.
- **Type check with `ty check`** — strict mode in CI.

## Style

- **NEVER `import *`.**
- **One public class/function per concept per module.** Files >500 lines get split.
- **Public API explicit via `__all__`** in `__init__.py`.
- **Docstrings on public functions/classes**: what, args, returns, raises.
- **Tests in `tests/`, mirror source layout.** `pytest` only. Use `pytest.mark.parametrize` for tables.

# C++ Rules
## Toolchain (Astral stack)

Default tools, all from [Astral](https://astral.sh). Pick alternatives only with a justified reason in the PR.

| Concern        | Tool       | Notes |
|----------------|------------|-------|
| Package mgmt   | `uv`       | env, install, lock, run; replaces pip/pip-tools/poetry |
| Lint           | `ruff check`  | replaces flake8, isort, pyupgrade, pylint subset |
| Format         | `ruff format` | replaces black |
| Type check     | `ty check`    | replaces mypy/pyright; fast, written in Rust |
| Python version | `.python-version` (managed by `uv`) | one source of truth |

## Typing

- **MUST use type hints on every public function and method.** Internal helpers should too unless trivial.
- **`ty check` must pass in strict mode.** CI fails otherwise. Treat `ty`'s warnings as errors in CI.
- **NEVER `Any` without a comment** explaining why no narrower type works.
- **`from __future__ import annotations`** at the top of every module.
- **Prefer `Protocol` and `TypedDict`** over `dict[str, Any]` and ad-hoc duck typing.
- **`Optional[X]` only when `None` is a real semantic state** ("absent" vs "empty"). Otherwise raise.

## Errors

- **NEVER bare `except:` or `except Exception:`** without re-raise. Catch the specific exception class.
- **NEVER swallow exceptions silently.** If you intentionally ignore one, log it at `debug` and add a comment.
- **Raise typed exceptions**, not `RuntimeError("everything's broken")`. Define a small hierarchy per package.
- **NEVER catch and re-raise as a different type** without `raise NewError(...) from original`.
- **`assert` is debug-only.** Don't use it for input validation; it's stripped under `python -O`.

## IO and resources

- **MUST use `with` blocks** for any resource with `close()` (`open`, sockets, db connections, locks).
- **`pathlib.Path`, never `os.path.join`** for new code.
- **NEVER `print()` in libraries or long-running scripts.** Use `logging` or `structlog`.
- **Logging config at the entry point only.** Libraries get a `logger = logging.getLogger(__name__)` and never `basicConfig`.
- **Read text with explicit encoding**: `open(path, encoding="utf-8")`. Defaults differ across platforms.

## Data shapes

- **Prefer `@dataclass(frozen=True, slots=True)`** for records. Use `pydantic` only when parsing untrusted input.
- **NEVER mutable default arguments.** `def f(x: list[int] = [])` is a bug. Use `None` and assign inside.
- **`Enum` (or `StrEnum` in 3.11+) for fixed string sets**, not bare strings.
- **f-strings only.** No `.format()`, no `%`.
- **`match` statements over long `if/elif` chains** when the dispatch is structural.

## CLIs

- **Use `typer` (default) or `click`.** Argparse only if you have a strong reason.
- **Every CLI has `--version`, `--help`, and exits non-zero on error** (typer/click do this for free if you let them).
- **Reserve exit codes**: `0` ok, `1` runtime error, `2` usage. Use codes >=64 for app-specific.
- **Provide `--dry-run`** for any command that writes/deletes/sends.
- **`if __name__ == "__main__":` calls a `main()` function**, not inline code.

## Async

- **NEVER mix sync and async carelessly.** `asyncio.run()` is the entry point — once.
- **Use `asyncio.TaskGroup`** (3.11+) for structured concurrency. Bare `asyncio.create_task` without an `await` later is a leak.
- **NEVER `time.sleep` in async code.** Use `asyncio.sleep`.
- **Cancellation: re-raise `asyncio.CancelledError`**, don't swallow. Clean up in `finally`.
- **Timeouts via `asyncio.timeout()`** (3.11+), not bespoke wall-clock checks.

## Packaging and tooling

- **`pyproject.toml` only.** No `setup.py`, no `requirements.txt` for new projects (committed lockfile via `uv` or `poetry` is fine).
- **`uv` is the default** package manager for new projects. Lockfile committed.
- **Pin a Python version** (`.python-version` file) so contributors and CI agree.
- **Format with `ruff format`, lint with `ruff check`** — both clean in CI.
- **Type check with `ty check`** — strict mode in CI.

## Style

- **NEVER `import *`.**
- **One public class/function per concept per module.** Files >500 lines get split.
- **Public API explicit via `__all__`** in `__init__.py`.
- **Docstrings on public functions/classes**: what, args, returns, raises.
- **Tests in `tests/`, mirror source layout.** `pytest` only. Use `pytest.mark.parametrize` for tables.