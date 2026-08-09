---
name: rust-standards
description: Look up the saya engineering standard for a topic and load only the relevant module. Use when unsure where code belongs, which test kind to use, how errors/APIs should look, the read-only/secrets rules, or the commit/PR workflow — or when the user asks "what's our standard for X" in the saya workspace.
---

# saya standards — reference loader

The standards are modular so you load **only** what a task touches. This skill maps a
topic to its module. Start from [`AGENTS.md`](../../../AGENTS.md) for the one-page
overview and the non-negotiables.

## Topic → module

| If the question is about… | Read |
| --- | --- |
| Where code lives · crate boundaries · dependency direction · adding a crate · **file-size cap** · "which crate?" · presentation vs. contract | [`architecture.md`](../../../docs/standards/architecture.md) |
| Error handling (`thiserror`, no `unwrap`/`panic`) · API design · naming · **which edition-2024 feature to use** (let-chains, async closures) · clippy/fmt/MSRV | [`rust-style.md`](../../../docs/standards/rust-style.md) |
| Writing/choosing tests · TDD loop · unit/integration/contract/live/**snapshot**/**property** · `nextest`/`insta`/`proptest` · fixtures · coverage expectations | [`testing.md`](../../../docs/standards/testing.md) |
| **Read-only SQL enforcement** · secrets & redaction · capability honesty · `cargo audit` · dependency policy | [`security.md`](../../../docs/standards/security.md) |
| Planning a change · **conventional commits** · PR checklist · review gates · CHANGELOG/ADR | [`workflow.md`](../../../docs/standards/workflow.md) |

## How to use

1. Match the task to a row above and open **only that module** (or the two it spans).
2. Apply the rule. Where it says *must*, treat it as binding — CI or `/rust-review`
   will reject a violation. Where it says *prefer*, deviate only with a one-line reason.
3. If a standard is wrong or missing, **fix the standard in the same change** as the
   code — a stale rule is worse than none.

For the full loop, the phase skills are `/plan-first`, `/tdd`, and `/rust-review`.
