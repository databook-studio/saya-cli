---
name: orchestrate-saya-release
description: Orchestrate delegated Saya implementation into an active release branch. Use for multi-domain changes, accepted-commit integration, or branch and worktree cleanup when Astra plans, Terra and Luna implement, Sol 6.1 reviews, and the lead session owns acceptance and merging. Do not use for a small direct edit or for external release publication.
---

# Orchestrate Saya Release Work

Keep the lead session accountable for the outcome while separating planning,
implementation, review, integration, and cleanup.

## Authority and boundaries

- Keep all work local unless the user separately authorizes a push, pull request,
  tag, publication, deployment, or other external action.
- Preserve dirty, frozen, comparison, and evidence worktrees. Never clean,
  reset, resolve, merge, or delete them merely because they look old.
- The active handover, task ledger, repository instructions, and exact release
  `HEAD` govern the campaign. Revalidate them before using an older packet.
- The lead session orchestrates: it owns decisions, task decomposition,
  acceptance, integration, gate execution, recovery records, and status. It
  does not take over a worker's implementation slice.
- Do not silently substitute models. If a requested model is unavailable or at
  capacity, report that lane as waiting and continue only with independent work.

## Model roles

- **Astra plans.** Use `gpt-6-astra` read-only to challenge scope, map the DAG,
  freeze interfaces, identify risks, assign disjoint ownership, and define the
  acceptance evidence. Astra does not edit product code.
- **Terra and Luna implement.** Use `gpt-5.6-terra` and `gpt-6-luna` in isolated
  worktrees created from the exact accepted release head. Give each writer one
  outcome and disjoint owned paths. Workers test-drive behavior, commit with a
  conventional message, and do not merge or clean other worktrees.
- **Sol 6.1 reviews.** Use `gpt-6.1-sol` read-only for correctness, security,
  test value, boundary preservation, and merge readiness. Findings must cite
  concrete files and lines. Route fixes back to Terra or Luna, then request a
  focused re-review.
- **The lead integrates.** Inspect every diff and report, run the owning proof,
  merge only accepted work into the active release branch, and run the combined
  gate on the merged tree.

## Campaign workflow

1. Inspect the release worktree, current `HEAD`, dirty state, active processes,
   worktrees, branches, handover, and task ledger. Record which state is current
   and which is historical.
2. Have Astra review or produce the implementation plan before writers start.
   Freeze shared interfaces, approval semantics, bounds, path ownership, test
   commands, and merge order. Serialize any shared seam.
3. Create one isolated branch and worktree per implementation lane plus a
   dedicated integration branch/worktree from the recorded release commit.
   Keep Cargo targets isolated unless the plan defines a safe shared target and
   serialized use. Do not assemble unaccepted work on the release branch.
4. Give Terra and Luna self-contained packets containing the invariant, failure
   mode, verified paths, owned paths, failing test expectation, required proof,
   exclusions, and exact base commit. One writer owns each file.
5. Inspect worker status and diffs before review. Freeze each writer, record the
   candidate commit and tree, and reject undeclared edits, weakened assertions,
   bypasses, unbounded work, hidden file-size violations, or claims supported
   only by zero matched tests.
6. Ask Sol 6.1 to review the exact recorded candidate diff against the frozen
   plan and repository standards. Findings remain open until fixed or explicitly
   dispositioned by the lead with evidence. A fix creates a new identity and
   requires the applicable proof and review again.
7. Run focused proof against each frozen candidate identity. For changed tests,
   apply the repository `test-audit` authoring gate. For user-visible behavior,
   exercise the real binary or owning adapter, not only a helper unit test.
   Recheck the commit and tree before integration and reject identity drift.
8. Merge reviewed candidates into the dedicated integration branch, normally
   with `git merge --no-ff`. Stop on conflicts and route substantive resolution
   to the owning implementation lane. Never merge into a dirty checkout.
9. Freeze the integrated commit and tree, then run the repository's complete
   local acceptance gate on that exact integration candidate. A focused green
   result is not final acceptance. Record exact commands, counts, failures, and
   exclusions. Any change after a gate invalidates that gate's acceptance.
10. Only after combined acceptance and an identity recheck, fast-forward the
    active release branch to the exact accepted integration commit. Update the
    task ledger and handover with release `HEAD`, accepted commits, review
    disposition, gate evidence, known limits, recovery refs, and next work.

## Cleanup after acceptance

Cleanup is a postcondition of successful integration, not part of review.

- For each merged candidate, verify release ancestry, no unique unmerged
  commits, patch equivalence when needed, branch/worktree ownership, and clean
  tracked status. Inventory untracked and ignored files, build outputs, nested
  repositories and Git links, reflog-only commits, the common Git directory,
  private worktree metadata, and every evidence file before removing anything.
- Create a complete recovery mapping: archive refs for commits, plus hashed and
  mode-preserving backups for unique files and private metadata that must be
  retained. Verify every recovery record from a fresh read before cleanup. A
  branch ref alone is never considered recovery for worktree-local files.
- Only after that verified postcondition, remove large build output with the
  repository-supported clean command, remove the clean worktree with Git, and
  delete its merged local branch using the safe deletion form. Prune stale
  worktree metadata afterward.
- Do not delete the release branch, the user's main checkout, dirty work,
  frozen comparisons, red binaries required by the ledger, or branches whose
  acceptance status is unresolved.
- Recount worktrees and local branches and record disk space after cleanup so
  the reduction is observable.

## Completion condition

The work is done only when the merged release tree passes its combined
acceptance gate, Sol 6.1 has no unresolved blocking finding, the ledger and
handover identify the exact accepted head, and eligible implementation branches
and worktrees have been safely removed. Report anything not run or not accepted
without upgrading it to complete.
