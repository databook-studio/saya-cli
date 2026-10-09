//! `[jobs]` resolution — the default budgets a run is declared with.
//!
//! The section holds what the run's specification and each step may leave
//! unset, so the engine can layer RunSpec and step budgets over these
//! resolved defaults per dimension. A ceiling left unset is unlimited at the
//! contract level — the engine enforces what is declared, pausing (never
//! silently overrunning) when a declared budget trips. There are no
//! environment overrides for `[jobs]` keys by design: run budgets must be
//! reproducible from the run's specification and config alone (plan G3).

use std::collections::BTreeMap;
use std::time::Duration;

use saya_types::{
    Budgets, MAX_BUDGET_ENDPOINTS, MAX_RUNNER_PROGRAMS, is_bare_name, is_name_shaped,
    is_refused_runner_program,
};

use crate::{
    ConfigError,
    model::{FetchJobsFile, InterpreterJobsFile, JobsFile, RunnerJobsFile},
};

/// Smallest accepted `[jobs]` ceiling. A zero on any dimension means "pause
/// before doing anything" — zero turns or zero tool calls stop the run
/// before its first model turn, a zero-second wall clock is the same instant
/// pause, zero tokens starve the endpoint on its first call, and zero
/// download bytes stop `http_download` before its first byte. As
/// *defaults* these are typos, not intents, so they are rejected rather than
/// silently clamped, matching the `context_byte_budget` discipline. No
/// upper bound: a long-running job may set any ceiling it needs, and the
/// unlimited case is "leave it unset".
const MIN_BUDGET: u64 = 1;

/// Provisional `[jobs.fetch]` default per-file download bound: large enough
/// for a corpus artifact, small enough that one URL cannot silently fill a
/// disk. Provisional until M5 measures real runs (U8).
pub(crate) const FETCH_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Provisional `[jobs.fetch]` default total run download bytes: several
/// files, bounded so one run cannot silently fill a disk. Provisional until
/// M5 measures real runs (U8).
pub(crate) const FETCH_MAX_RUN_BYTES: u64 = 1024 * 1024 * 1024;

/// Provisional `[jobs.fetch]` default per-request timeout: matches the
/// `[ai] timeout_seconds` default. With resumable partials a longer
/// transfer is a sequence of budgeted attempts. Provisional until M5
/// measures real runs (U8).
pub(crate) const FETCH_TIMEOUT_SECONDS: u64 = 60;

/// Provisional `[jobs.runner]` default wall-clock ceiling for one child
/// process: minutes of headroom for a data program, tight enough that a
/// wedged child cannot hold a step open for an hour. Provisional until M5
/// measures real runs (U8).
pub(crate) const RUNNER_TIMEOUT_SECONDS: u64 = 300;

/// The `[host_commands]` default per-call ceiling in seconds: the executor's
/// own conservative number, the same value a user-layer section resolves to
/// when it declares nothing. A declared zero is a typed resolve error.
pub(crate) const HOST_COMMANDS_TIMEOUT_SECONDS: u64 = 600;

#[path = "jobs/host.rs"]
mod host;
#[path = "jobs/resolution.rs"]
mod resolution;
/// Effective download budget defaults, resolved from `[jobs.fetch]`.
/// Always concrete: a run with nothing declared is bounded by these
/// conservative defaults rather than unbounded — the point of the download
/// budget is that it exists without being asked for.
#[path = "jobs/resolved.rs"]
mod resolved;

use host::{require_at_least_one, resolve_token_map};
pub(crate) use host::{require_max_iterations, resolve_host_commands};
pub(crate) use resolution::resolve;
pub(crate) use resolved::resolve_session_deny;
pub use resolved::{
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs,
    ResolvedRunnerJobs, ResolvedSessionDeny,
};
