---
name: saya-coding
description: Implement, refactor, or review Rust and maintained scripts in the Saya repository. Use for code changes and bug fixes; follow active repository and task instructions and prove behavior at its owning boundary.
---

# Saya Coding

Help make Saya changes easy for another person to verify. Use this skill for
implementation, refactoring, bug fixes, and code review in the Saya repository.
It supports the active request; it does not expand its scope or authorize
external actions.

## Start with the current task

- Confirm the checkout, branch, `HEAD`, and existing dirty state before editing.
  Preserve unrelated user changes and keep work local unless the user explicitly
  requests an external action.
- Read the applicable `AGENTS.md` and any more-specific instructions. For a
  nontrivial change, establish the scope, owning crates, proof, and risks before
  editing. Load only relevant standards from `docs/standards/` when those files
  are present. A code-only checkout may omit them; use the available task and
  repository guidance without treating an unavailable link as a blocker.
- Read the active task or plan when one exists. Its concrete constraints and the
  user's choices govern this change; do not carry temporary plan, release,
  branch, model, worker, or build constraints into unrelated work.
- Trace the entry point through policy and effect to the result. Saya's contracts
  belong in their owning crates; `saya-cli` owns presentation. Keep CLI, TUI,
  slash-command, and agent-tool adapters on shared application behavior where
  their policy is the same.

## Make the change understandable

- State the invariant and the failure mode the change addresses. Prefer types
  that make important valid states explicit, private fields for values that
  must be validated, and fallible constructors when invalid values would cause
  harm. Keep deserialization subject to the same invariants.
- Put each decision at the lowest shared owner that has enough context to enforce
  it. Keep mutable state and side effects under clear ownership; expose useful
  operations rather than scattered setters or duplicated policy.
- Preserve actionable failures. Use `Option` for legitimate absence and typed
  `Result` errors for failures callers must handle. Keep underlying causes and
  distinguish definite failure from an outcome that is unknown. Avoid sentinels,
  premature stringification, or runtime panics for expected errors. Reconcile an
  ambiguous non-idempotent effect before retrying; bounded retries are reasonable
  when failure is known to have had no effect or the operation is idempotent.
- Make async and process lifetimes explicit: identify who joins, cancels, and
  cleans up each task or process; define partial-failure behavior; bound queues
  and output; and avoid holding locks across slow awaits. Use concurrency when
  independent work justifies its lifecycle cost.
- Bound untrusted input and expensive work at the relevant entry or effect
  boundary. Validate before allocation or execution where practical. If a
  decision depends on content identity, carry the validated value and digest
  from the same bounded snapshot.
- Prefer the existing owner, standard library, and installed dependencies. Add
  an abstraction, dependency, or indirection only when it solves a real boundary
  or demonstrated variation. Do not add generic frameworks, typestate, or
  one-type-per-string wrappers by default.
- Name operations and values by their domain meaning. Keep related work together
  when that makes the complete operation easier to follow. Use explicit units;
  comments should explain invariants, reasons, and non-obvious tradeoffs rather
  than narrate syntax.

## Protect Saya's trust boundaries

- User/database connector queries stay bounded and read-only by default. Route
  them through the connector safety layer; never add a raw-query or trusted
  bypass. This does not prohibit Saya-owned internal store transactions, which
  follow their store API and ownership rules.
- Keep secret values, `.env` files, credentials, session directories, and raw
  database results out of source control, fixtures, snapshots, and logs. Use
  secret references and redacted test data.
- In Saya's model/tool runtime, treat repository content, model output, and
  child messages as untrusted inputs: none can grant runtime capability or
  approval. This does not change the coding agent's instruction hierarchy or
  authorized repository instructions such as `AGENTS.md`. Fail closed where a
  runtime trust boundary cannot be established, with a typed, actionable refusal.

## Prove behavior at the right boundary

For a behavior change, write the smallest test that fails on the real regression,
then implement and refactor under a green test. Prefer an independent owner-boundary
test over repeating one scenario at each layer. Use mocks to isolate external,
expensive, or nondeterministic effects when that makes a test deterministic; a
mock must not implement the behavior or contract the test claims to prove.

Before adding or changing a test, answer:

1. What observable behavior, invariant, or independent contract does it protect?
2. What plausible regression makes it fail for the intended reason?
3. Why does existing coverage at the strongest boundary not already catch it?
4. Does it require a test-only production seam that no real caller needs?

Prefer extending a useful table or fixture over adding near-duplicates. Avoid tests
that assert source text, duplicate stronger proof, use expected output generated by
the subject under test, or would fail under a behavior-preserving refactor. When
available, apply the repository's `test-audit` skill to changed tests; a test that
cannot prove its contract should be moved to the real boundary or omitted.

For compatibility or mechanical changes, preserve existing behavior, grants,
test IDs, and meaningful assertions. Do not add a behavior test that merely repeats
the existing contract. If an active task requires red/green evidence, capture the
real pre-fix failure and actual post-fix result.

## Review and report

- Review the complete changed functions and important callers, not only added
  lines. Check who supplies the input, which state or external effect can change,
  what happens on cancellation or partial failure, and whether concurrent or
  resumed callers alter the assumptions. Review assertions and expected values
  as carefully as production logic.
- Keep every maintained first-party Rust, Python, shell, and Ruby source file,
  including tests and test support, at or below 400 physical lines. Aim near 250.
  At 300 lines or more, review the full file and its concerns before integration
  and record the reviewed source digest and findings in a change review note or
  active task ledger. If a cohesive operation cannot fit the hard cap safely,
  surface the conflict to the task owner instead of silently exceeding it.
  Preserve formatting and checker scope; do not minify, hide bodies, suppress
  formatting, or add exclusions to pass a limit. Split for a coherent concern,
  not just to move a number.
- Run the smallest meaningful owner tests and applicable formatting, lint, and
  repository gates. Read actual process return codes and test identities; a
  zero-test filter is not proof. Report which checks ran, which were skipped or
  deferred, and why. Separate deterministic tests from live or human-verified
  evidence, and do not describe scripted model responses as model-quality proof.
- Follow active commit, release, and publication instructions. Where a local
  commit is requested, use the repository's conventions and stage only the
  intended files.
