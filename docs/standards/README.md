# saya engineering standards

The rules that let humans and AI agents change this codebase safely, consistently,
and without re-litigating decisions. These docs are **modular on purpose**: one
concern per file so a contributor — or an agent — loads only what the task touches.

The one-page front door for agents is [`/AGENTS.md`](../../AGENTS.md) (read by
Claude Code via the `CLAUDE.md` symlink and by Cursor via `.cursor/rules/`). It
points here for detail.

## The golden path

Every non-trivial change follows the same loop. The workflow skills automate it.

1. **Plan first** — write down scope, the crates touched, the tests you'll add, and
   the risks *before* editing code. (`/plan-first`, [workflow.md](workflow.md))
2. **Test-drive** — red → green → refactor: a failing test first, the smallest code
   to pass, then clean up under green. (`/tdd`, [testing.md](testing.md))
3. **Review** — check the diff against these standards before you commit.
   (`/rust-review`, [workflow.md](workflow.md))

## The modules

| Load this when you're… | Doc |
| --- | --- |
| Deciding where code lives, adding a crate, or splitting a file | [architecture.md](architecture.md) |
| Writing Rust — errors, APIs, which 2024-edition features to reach for | [rust-style.md](rust-style.md) |
| Adding or changing tests, choosing a test kind, running the suite | [testing.md](testing.md) |
| Touching SQL execution, secrets, redaction, or capabilities | [security.md](security.md) |
| Planning a change, committing, or opening a PR | [workflow.md](workflow.md) |

## How agents should use these docs

- **Don't load everything.** Read `AGENTS.md`, then open only the module(s) the task
  touches. `/rust-standards <topic>` resolves a topic to the right file.
- **These are binding, not advisory.** Where a rule says *must*, CI or review will
  reject a violation. Where it says *prefer*, deviate only with a one-line reason in
  the PR.
- **When a standard is wrong, change the standard** — in the same PR, with the code.
  A stale rule that everyone silently ignores is worse than no rule.

## Non-negotiables (the short list)

These appear in full in the modules; they are collected here because breaking one is
never a judgment call.

- **Read-only by default.** All SQL goes through the safety layer
  ([security.md](security.md)). Never add a bypass.
- **No secrets in the tree.** No `.env`, keys, session dirs, or raw DB results in
  commits or fixtures — use secret *references* and redacted fixtures.
- **Behavior changes start with a failing test.** ([testing.md](testing.md))
- **Files stay small.** New production `.rs` files ≤ 150 lines (soft), 250 hard cap.
- **Contracts live in their crate; presentation lives in `saya-cli`.**
