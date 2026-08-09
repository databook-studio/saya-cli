# Rust style standard

Idiomatic, modern Rust for this workspace. We target **edition 2024** with **MSRV
1.88** (`clippy.toml`, `rustfmt.toml`, and CI enforce this). Every feature named
below is stable at or before 1.88 — safe to use without raising the MSRV.

## Formatting & lints (enforced by CI)

- `cargo fmt` is law (`rustfmt.toml`: edition 2024, `max_width = 100`). No hand
  formatting that `fmt` would undo.
- `cargo clippy --workspace --all-targets -- -D warnings` must pass. **Warnings are
  errors.** Don't `#[allow(...)]` to silence a lint without a `// ` reason on the same
  line explaining why the lint is wrong *here*.
- Raising the MSRV is a deliberate, documented change (update `clippy.toml`, workspace
  `rust-version`, and the CI matrix together) — never a side effect.

## Edition-2024 features to prefer

| Reach for | Since | Instead of |
| --- | --- | --- |
| **let-chains** — `if let Some(x) = a && x.ok() && let Ok(y) = f(x)` | 1.88 | nested `if let` / `match` pyramids |
| **async closures** — `async \|x\| { … }`, `AsyncFn*` bounds | 1.85 | `\|x\| async move { … }` returning a boxed future |
| `let ... else { return; }` | 1.65 | early-return `match`/`if let` boilerplate |
| RPIT / `impl Trait` in return position & args | — | boxing when a concrete opaque type suffices |
| `Option::is_none_or` / `is_some_and` | 1.82 / 1.70 | verbose `map_or(true, …)` (already used in `read_only.rs`) |

- `gen` is a **reserved keyword** in edition 2024 — don't use it as an identifier.
  Generators/`gen` blocks are not stable; don't reach for them yet.
- Use new features because they make the code *clearer*, not for novelty. A let-chain
  that replaces a three-deep `if let` is a win; one that packs five unrelated
  conditions onto a line is not.

## Error handling

- **Library crates return typed errors, never panic on expected failure.** Model
  errors with `thiserror` (see `saya_types::ConnectionError`, `saya_config` errors).
- **No `unwrap()`, `expect()`, or `panic!` in library crate code paths** reachable
  at runtime. Allowed only in: tests, `build.rs`, and cases with a *proven* invariant
  documented in an `expect("why this cannot fail")` message.
- Prefer `?` and `map_err` to convert at boundaries. Don't stringify an error early and
  lose its structure; convert to the crate's error type.
- `saya-cli` is the only crate that turns errors into user-facing text — and it does so
  through `render`, not `println!`.

## API design

- Public enums that model an open-ended domain (events, error kinds, dialects) are
  `#[non_exhaustive]` so adding a variant isn't a breaking change.
- Prefer borrowed parameters (`&str`, `&[T]`) and owned returns. Take `impl Into<String>`
  for constructors that store a `String` (see `QueryResult::empty`).
- Derive `Debug` on public types; derive `Clone`/`PartialEq` when cheap and useful for
  tests. Keep `serde` attributes (`rename_all`, `skip_serializing_if`) consistent with
  the existing wire types in `render` and `saya-agent`.
- Keep function signatures honest: return `Result` when it can fail, not a sentinel;
  return `Option` for absence, not an empty-string convention.

## Naming & idioms

- `snake_case` items, `CamelCase` types, `SCREAMING_SNAKE_CASE` consts. Follow the
  Rust API Guidelines for conversions (`as_`, `to_`, `into_`) and getters (no `get_`
  prefix).
- Constructors that can fail return `Result`; infallible ones are `new`/`from`.
- Keep `use` groups ordered as `rustfmt` leaves them; import types, call free functions
  by short path.
- Comments explain **why**, not what. Match the surrounding density — this codebase
  comments sparingly and precisely (see the CI-env comment in `ci.yml`).
