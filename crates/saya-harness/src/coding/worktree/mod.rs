//! Managed worktree checkouts: durable create orchestration under the
//! app-owned `worktrees` root.
//!
//! [`create_managed_checkout`] walks one checkout id through the R14 state
//! machine under a per-`(job, checkout)` in-process lock held across the
//! whole read–effect–outcome run, honoring the ledger invariant that
//! exactly one outcome is ever recorded per create operation.
//! Cross-process serialization is not provided; the store itself enforces
//! one durable outcome. The lock map holds weak entries that are pruned on
//! every insert, so its bound is the number of in-flight keys, not settled
//! checkouts.
//!
//! A checkout id is owned by one job for ever: the first create that
//! durably records the id owns it, the managed path derives from the id
//! alone, and any later create for an id another job owns is refused with
//! `CheckoutIdInUse` before anything is read or sequenced.
//!
//! 1. Pre-validate the plan against the published snapshot without touching
//!    the ledger: the plan digest must equal the published snapshot's
//!    `plan_manifest_sha256` and the captured snapshot id must equal the
//!    plan's, else `PlanDigestMismatch`.
//! 2. Read the durable record (`get_managed_checkout`):
//!    - absent: preflight the store's predictable intent refusals — the job
//!      must be under its 256-checkout cap (`CheckoutCapReached`) and
//!      running (`JobNotRunning`) — then append the durable create intent
//!      through the job event sequencer and continue with the allocated
//!      operation;
//!    - present: validate the caller input against the recorded ownership
//!      (base commit, snapshot id and captured digest, repository and
//!      common-dir identity, managed path, agent); any difference is
//!      `InputMismatch`, recorded nowhere. Every replay then uses the
//!      recorded values, never the caller's — including the recorded base
//!      commit for adopt and materialize;
//!    - `Creating`: retry — skip the intent, reuse the recorded intent
//!      operation as the open operation, continue at step 4;
//!    - `Active`: idempotent replay — adopt the existing worktree and return
//!      [`CreatedCheckout`] with `receipt: None`;
//!    - `Failed`/`Uncertain`/`Retained`/`Removing`/`Removed` (and any future
//!      lifecycle label): `Terminal` — the id is settled, use a new one.
//! 3. Path state under the worktrees root: missing path →
//!    `GitService::add_worktree_no_checkout`; present path →
//!    `GitService::adopt_worktree` (identity re-verified inside). A
//!    `MutationOutcomeUnknown` here records `Uncertain`.
//! 4. On a `Creating` retry, first reconcile a target a crash may have left
//!    populated: `reset_materialization_target` returns it to the
//!    `.git`-only state (safe — the checkout is not `Active`, so nothing
//!    has been handed out) before `materialize_snapshot`; a reset that
//!    cannot clean up records `Failed` like a failed materialize.
//! 5. Persist the plan bytes: `Recorded`/`AlreadyPresentSame` are fine; a
//!    digest `Conflict` records `Failed`.
//! 6. Record the outcome and return.
//!
//! Outcome classification — the intent stays open and recoverable whenever
//! nothing is recorded:
//!
//! - **Failed** (terminal): `SnapshotError::MaterializeCleanupFailed` (the
//!   target must be discarded; from the retry reset or the materialize
//!   itself), plan-bytes `StoreError::Conflict`,
//!   `GitError::InvalidBaseCommit`, `GitError::ManagedPathOutsideState`.
//! - **Uncertain**: `GitError::MutationOutcomeUnknown` from the worktree
//!   add.
//! - **Transient** (no outcome recorded): store unavailability, cancellation
//!   (`GitError::Cancelled`/`SnapshotError::Cancelled`), snapshot digest and
//!   IO failures, adoption refusals, and sequencer failures. The typed error
//!   is returned; a retry with the same checkout id resets the target to
//!   `.git`-only, adopts the worktree, and re-materializes.
//! - **Refusals** (no outcome recorded, nothing to clean up):
//!   `PlanDigestMismatch`, `InputMismatch`, `JobNotRunning`,
//!   `CheckoutCapReached`, `OperationCapReached`, and `CheckoutIdInUse` are
//!   refused before the ledger is touched, so they never poison it;
//!   `Terminal` is the settled-id refusal.
//!
//! `Completed` is recorded only after every step succeeded. Recording a
//! terminal outcome is best-effort: a journaling failure never masks the
//! classified primary error, and the still-open intent is re-classified on
//! the next replay. A recorded `Failed`/`Uncertain` settles the checkout id
//! permanently; recovery needs a new id. The original repository is never
//! written: every git effect flows through `GitService` against the common
//! directory only — no marker files, no `.gitignore` mutation, no
//! shared-write fallback, and ownership stays DB-only (R14 row + live git
//! identity + artifacts digest; a path alone is never authority).
mod create;

mod create_error;

mod create_events;

mod create_preflight;

mod confinement;

mod environment;

mod ownership;

pub use confinement::{ConfinedCheckout, ConfinementError, confine_checkout};
pub use create::{CreateCheckoutInput, CreatedCheckout, create_managed_checkout};
pub use create_error::CreateCheckoutError;
pub use environment::{ChildEnvironment, ChildEnvironmentError, ChildNetworkPolicy};
pub use ownership::{OwnershipError, OwnershipInput, VerifiedCheckout, verify_owned_checkout};

mod cleanup;
mod reconcile;
mod repair;
mod retention;
pub use cleanup::{CleanupError, CleanupInput, cleanup_managed_checkout};
pub use reconcile::{CheckoutReconciliation, reconcile_managed_checkout};
pub use retention::{RetainedCheckout, retain_managed_checkout};

mod retention_budget;
pub use retention_budget::ensure_retention_capacity;
