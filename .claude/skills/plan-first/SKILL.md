---
name: plan-first
description: Write a short implementation plan before touching code in the saya workspace — scope, crates affected, the tests you'll add, and risks. Use before starting any non-trivial change, feature, or refactor (e.g. "plan this change", "before we implement", "how should we approach X"), and whenever the user asks you to plan first.
---

# Plan first

The first phase of saya's golden path (see [`docs/standards/workflow.md`](../../../docs/standards/workflow.md)):
**never edit code before the plan is written.** For anything beyond a typo or a
one-line fix, produce the plan below, then get sign-off on user-facing work before
implementing.

## Steps

1. **Understand the request and the code it touches.** Read the relevant crate(s) and
   the standards module for the area ([`docs/standards/`](../../../docs/standards/README.md)).
   Reuse existing functions/patterns — don't invent new ones where one exists.
2. **Write the plan** with these sections (keep it scannable):
   - **Context** — the problem/need and the intended outcome.
   - **Scope** — what changes; explicitly what does *not*.
   - **Crates touched** — and why, respecting the dependency direction
     ([architecture.md](../../../docs/standards/architecture.md)). Presentation only in
     `saya-cli`; contracts in their own crate.
   - **Test list** — the tests you'll add/change and the *kind* of each (unit /
     integration / contract / live / snapshot / property, per
     [testing.md](../../../docs/standards/testing.md)). Name the **red** tests before
     any code exists.
   - **Risks & rollback** — what could break; how it's reverted.
3. **Check it against the non-negotiables** in [`AGENTS.md`](../../../AGENTS.md):
   read-only SQL, no secrets in the tree, file-size cap, TDD.
4. **Get sign-off** for user-facing features before implementing. Then hand off to
   `/tdd`.

## Guardrails

- No edits during planning — this phase is read-only exploration + writing the plan.
- A plan whose test list is empty is not a plan for a behavior change. If nothing is
  testable, say why.
- Prefer the smallest change that satisfies the request; flag scope creep instead of
  absorbing it silently.
