//! Background SQL execution for direct commands (/sql, /export, /chart,
//! /explain). Each command runs on its own worker thread with its own
//! current-thread tokio runtime, returning a non-blocking `mpsc` receiver the
//! event loop polls each tick. The point is narrow and worth stating plainly:
//! the event loop never blocks on a query. That is what the worker buys.
//!
//! What it does **not** buy, and the older module doc wrongly claimed:
//!
//! - **No live spinner by itself.** The status-bar spinner advances only when
//!   `App::is_busy()` is true, so visibility depends on `is_busy()` covering a
//!   running SQL task — which it now does. The label and elapsed time reuse
//!   the agent's `request.started` / `request.activity` fields (a SQL task and
//!   an agent stream are never concurrent: the queued-prompt gate prevents
//!   dispatch while either is busy).
//! - **Cancellation is a request, not settlement.** Esc signals the operation
//!   token and drops the receiver so the UI moves on. The worker keeps owning
//!   native setup/query work and its permit until that work settles.
//! - **No silent replacement.** A second command submitted while one runs is
//!   held by the queued-prompt gate until the first finishes (both results
//!   report). The dispatch handler additionally refuses a `SqlTask` that
//!   reaches it while one is already running (`App::admit_second_sql`).
//! - **No unbounded workers.** Every spawn holds one process-wide worker
//!   permit (`super::worker_permits`) from just before the thread starts
//!   until its thread function returns, so detached workers still count
//!   against the cap and admission refuses when all are held.
//!
//! Dispatch shares the existing `Arc<RuntimeConfig>` with the worker rather
//! than deep-cloning the resolved config (plaintext secrets included) per
//! command. Dispatch captures everything the *completion* step needs
//! (follow-up kind, target path, connection) at command time; the loop polls
//! the channel and calls [`complete`] when the result lands.

mod complete;
mod spawn;
mod task;

pub(crate) use complete::complete;
pub(crate) use spawn::spawn;
pub(crate) use task::{Followup, SqlTask};
