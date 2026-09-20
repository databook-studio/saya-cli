//! Second-SQL admission guard: refuse a direct-SQL command that arrives while
//! one is already running, instead of silently replacing (and dropping) it.

use super::super::types::App;

impl App {
    /// Decides whether a direct-SQL command may start now. The primary defence
    /// against silent result loss is the queued-prompt gate: `is_busy()` now
    /// covers SQL tasks, so a second command submitted while one runs is held
    /// in `pending` and dispatches only after the first finishes (both results
    /// report). This guard is the backstop: should a `SqlTask` ever reach the
    /// dispatch handler while one is already running, it is refused with a
    /// message instead of replacing the first receiver.
    pub(crate) fn admit_second_sql(&self) -> super::SecondSqlDecision {
        if self.sql_task.is_some() {
            super::SecondSqlDecision::Reject(
                "A SQL command is already running — wait for it to finish before starting another.",
            )
        } else {
            super::SecondSqlDecision::Start
        }
    }

    /// Detaches the in-flight SQL command so the UI moves on without blocking.
    /// The worker thread is not joined and the connector has no cancellation
    /// token wired here, so the query keeps running **server-side**; its result
    /// lands on a dropped channel and is discarded. The message says exactly
    /// that — it never claims the query was cancelled.
    pub(crate) fn detach_sql_task(&mut self) {
        if let Some((_, _, started)) = self.sql_task.take() {
            // Release the status fields the bar reused while the query ran.
            self.request.started = None;
            self.request.activity = None;
            self.transcript.push(
                super::super::transcript::BlockKind::System,
                format!(
                    "Detached the running query ({}s elapsed) — it may still be running on the \
                     server; its result will be discarded.",
                    started.elapsed().as_secs()
                ),
            );
        }
    }
}

/// The dispatch decision when a second direct-SQL command arrives while one is
/// already running. A pure function over [`App`] state so the result-loss
/// behaviour is testable without a live database (see [`App::admit_second_sql`]).
pub(crate) enum SecondSqlDecision {
    /// No SQL command is in flight — start this one.
    Start,
    /// One is already running — refuse rather than silently drop the first
    /// result. The message is shown to the user.
    Reject(&'static str),
}
