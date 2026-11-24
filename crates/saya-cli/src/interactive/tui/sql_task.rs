//! One bounded, read-only query on a worker thread.
//!
//! The new surface's `/sql` runs today's worker rather than a second one: the
//! query goes through the safety layer with a standing worker permit, and the
//! answer comes back as the worker's own terminal event. The point is narrow
//! and worth stating plainly: nothing the surface draws waits on a query.
//!
//! - **Cancellation is a request, not settlement.** The surface's cancel signals
//!   the operation token; the worker keeps owning its native setup/query work
//!   and its permit until that work settles.
//! - **No unbounded workers.** Every spawn holds one process-wide worker permit
//!   (`super::worker_permits`) from just before the thread starts until its
//!   thread function returns, so a detached worker still counts against the cap
//!   and admission refuses when all are held.
//!
//! The old surface's completion path — pushing a finished result into its
//! transcript, exporting or charting it from there — went with it in X1; the new
//! surface files the result in its own preview. What is shared is the query.

mod spawn;
mod task;

pub(crate) use spawn::spawn;
pub(crate) use task::SqlTask;

/// The most recent query the surface ran (via `/sql` or an agent tool), so
/// `/export` and `/chart` can re-run it and a bare `/investigation save` can
/// state it. Re-running a SELECT stays read-only.
///
/// It lived on the old TUI's `App` until X1 deleted that app; the fact is the
/// query worker's, so the new surface reads it from the worker's module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LastQuery {
    pub(crate) sql: String,
    pub(crate) connection: Option<String>,
}
