//! The `runs` repository: metadata rows for runs and their steps, created by
//! migration step 7.
//!
//! Payload-free by construction. A run's goal, plan, and event journal live
//! in the run directory the engine owns; no parameter of this API accepts
//! goal or plan text, so a sentinel planted in a goal cannot reach these
//! bytes (`tests/runs.rs` scans for exactly that, over the database and its
//! WAL sidecars). The rows carry identity, status, timestamps, typed failure
//! codes, capability flags, and numeric budget/usage figures — the mirror the
//! engine updates at every transition and the surface `saya run list` reads.
//!
//! Usage is nullable end to end: an unreported figure is NULL in the row and
//! `None` in the API — "unknown", never zero.
//!
//! The engine is the state machine's authority; this store is its second
//! guard, refusing a transition the machine does not allow instead of
//! recording one.

mod reads;
mod records;
mod status;
mod writes;

pub use records::{
    NewRun, RunBudgets, RunCapabilityFlags, RunRecord, RunStepRecord, RunSummary, RunUsage,
};
pub use status::{RunStatus, RunStepStatus};

use async_trait::async_trait;
use saya_types::{RunFailureCode, RunId};

use crate::{SqliteStateStore, StoreError};

/// The run-spine repository over the `runs` and `run_steps` tables.
#[async_trait]
pub trait RunStore: Send + Sync {
    /// Create a run's metadata row. The run starts `Planned`; approval is a
    /// transition, never a creation parameter. A duplicate id is `Conflict` —
    /// a resumed run is loaded, not re-created.
    async fn create_run(&self, run: NewRun) -> Result<RunRecord, StoreError>;

    /// One run's full metadata row, or `None` when the id is unknown.
    async fn get_run(&self, id: &RunId) -> Result<Option<RunRecord>, StoreError>;

    /// Every run's summary, most recent first.
    async fn list_runs(&self) -> Result<Vec<RunSummary>, StoreError>;

    /// Move a run through the state machine (`planned → approved → executing
    /// ⇄ paused → completed | failed | cancelled`). An illegal transition is
    /// `Conflict`; `Failed` requires its typed code and no other status
    /// carries one.
    async fn set_run_status(
        &self,
        id: &RunId,
        status: RunStatus,
        failure_code: Option<RunFailureCode>,
    ) -> Result<(), StoreError>;

    /// Replace the run's accumulated usage. Mirror semantics: the engine
    /// sends the totals it holds; an unreported figure is `None` — unknown,
    /// never zero — and stays NULL.
    async fn set_run_usage(&self, id: &RunId, usage: RunUsage) -> Result<(), StoreError>;

    /// Insert or transition a step. A step's first sighting is pending or
    /// running — never already finished; afterwards the step machine governs,
    /// including `failed → running` for the engine's bounded retry.
    async fn upsert_step(
        &self,
        run: &RunId,
        step: usize,
        status: RunStepStatus,
    ) -> Result<(), StoreError>;

    /// Replace a step's accumulated usage, same mirror semantics as the run's.
    async fn set_step_usage(
        &self,
        run: &RunId,
        step: usize,
        usage: RunUsage,
    ) -> Result<(), StoreError>;

    /// Every step of one run, in plan order — the resume scan reads this.
    async fn list_steps(&self, run: &RunId) -> Result<Vec<RunStepRecord>, StoreError>;
}

#[async_trait]
impl RunStore for SqliteStateStore {
    async fn create_run(&self, run: NewRun) -> Result<RunRecord, StoreError> {
        writes::create_run(self, run).await
    }

    async fn get_run(&self, id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        reads::get_run(self, id).await
    }

    async fn list_runs(&self) -> Result<Vec<RunSummary>, StoreError> {
        reads::list_runs(self).await
    }

    async fn set_run_status(
        &self,
        id: &RunId,
        status: RunStatus,
        failure_code: Option<RunFailureCode>,
    ) -> Result<(), StoreError> {
        writes::set_run_status(self, id, status, failure_code).await
    }

    async fn set_run_usage(&self, id: &RunId, usage: RunUsage) -> Result<(), StoreError> {
        writes::set_run_usage(self, id, usage).await
    }

    async fn upsert_step(
        &self,
        run: &RunId,
        step: usize,
        status: RunStepStatus,
    ) -> Result<(), StoreError> {
        writes::upsert_step(self, run, step, status).await
    }

    async fn set_step_usage(
        &self,
        run: &RunId,
        step: usize,
        usage: RunUsage,
    ) -> Result<(), StoreError> {
        writes::set_step_usage(self, run, step, usage).await
    }

    async fn list_steps(&self, run: &RunId) -> Result<Vec<RunStepRecord>, StoreError> {
        reads::list_steps(self, run).await
    }
}
