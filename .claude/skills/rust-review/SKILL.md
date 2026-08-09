---
name: rust-review
description: Review a diff in the saya workspace against the engineering standards before committing — style, tests, security, architecture, file-size, commit hygiene. Use after implementing a change, when asked to review code / a diff / a PR for saya, or as the final phase before committing. Runs the local gate and checks docs/standards.
---

# saya code review

The third phase of the golden path: review the diff **against the standards, not
personal taste**, before it's committed. Standards:
[`docs/standards/`](../../../docs/standards/README.md).

## Steps

1. **See the diff** — `git diff` (or `git diff main...HEAD`). Review only what changed
   plus the code it directly affects.
2. **Run the gate** — `.claude/skills/rust-review/scripts/review-gate.sh` (fmt, clippy,
   nextest, doctests; audit when deps changed). Everything must be green.
3. **Walk the checklist** below, citing the standards module for any finding. Report
   findings most-severe first; a passing gate is necessary but not sufficient.

## Checklist

- **Plan honored** — diff matches the agreed scope; no unrelated drive-by changes.
- **Tests** ([testing.md](../../../docs/standards/testing.md)) — a test that would fail
  on `main` covers the change; the right *kind*; bug fixes add a regression test;
  **safety-layer changes have property/security tests**.
- **Style** ([rust-style.md](../../../docs/standards/rust-style.md)) — `fmt`/`clippy -D
  warnings` clean; no unexplained `#[allow]`; **no `unwrap`/`expect`/`panic` in library
  paths**; errors typed via `thiserror`; sensible use of edition-2024 features.
- **Architecture** ([architecture.md](../../../docs/standards/architecture.md)) — right
  crate; dependencies point down; **presentation only in `saya-cli`**; new/production
  `.rs` files ≤ 150 lines soft / 250 hard.
- **Security** ([security.md](../../../docs/standards/security.md)) — all SQL routes
  through `saya-connectors/src/safety/`, **no bypass**; no secrets/`.env`/session
  dirs/raw results committed; redaction covers new persisted fields.
- **Docs & commits** ([workflow.md](../../../docs/standards/workflow.md)) — new
  flags/config/behavior documented; `CHANGELOG.md` updated; **conventional commit**,
  no `Co-Authored-By` trailer; alpha limitations stated honestly.

## Output

For each issue: the file:line, which standard it breaks, and the concrete fix. If the
diff is clean, say so and name what you verified — don't invent problems.
