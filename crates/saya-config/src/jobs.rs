//! `[jobs]` resolution — the download budgets a run's `http_download` calls
//! spend from.
//!
//! The run's own ceilings (wall clock, turns, tool calls, tokens per endpoint)
//! live in `[limits.run]`, one vocabulary with every other surface's; what is
//! left here is `[jobs.fetch]`, which is a download budget with concrete
//! defaults rather than a ceiling the run declares. There are no environment
//! overrides for `[jobs]` keys by design: a run's budgets must be reproducible
//! from the run's specification and config alone.

use saya_types::is_bare_name;

use crate::{
    ConfigError,
    model::{FetchJobsFile, JobsFile},
};

/// Smallest accepted value for a setting that counts things: a download
/// byte ceiling, a download or host-command timeout, or a run's per-endpoint
/// token ceiling. A zero on any of them means "stop before doing anything" —
/// zero download bytes stop `http_download` before its first byte, a
/// zero-second timeout is the same instant pause, and zero tokens starve the
/// endpoint on its first call. As *defaults* these are typos, not intents, so
/// they are rejected rather than silently clamped, matching the
/// `context_byte_budget` discipline. No upper bound: a long-running job may
/// set any ceiling it needs, and the unlimited case is "leave it unset".
const MIN_BUDGET: u64 = 1;

/// Provisional `[jobs.fetch]` default per-file download bound: large enough
/// for a corpus artifact, small enough that one URL cannot silently fill a
/// disk. Provisional until real runs are measured.
pub(crate) const FETCH_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Provisional `[jobs.fetch]` default total run download bytes: several
/// files, bounded so one run cannot silently fill a disk. Provisional until
/// real runs are measured.
pub(crate) const FETCH_MAX_RUN_BYTES: u64 = 1024 * 1024 * 1024;

/// Provisional `[jobs.fetch]` default per-request timeout: matches the
/// `[ai] timeout_seconds` default. With resumable partials a longer
/// transfer is a sequence of budgeted attempts. Provisional until real runs
/// are measured.
pub(crate) const FETCH_TIMEOUT_SECONDS: u64 = 60;

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

pub(crate) use host::{require_at_least_one, resolve_host_commands};
pub(crate) use resolution::resolve;
pub(crate) use resolved::resolve_session_deny;
pub use resolved::{ResolvedFetchJobs, ResolvedHostCommands, ResolvedJobs, ResolvedSessionDeny};
