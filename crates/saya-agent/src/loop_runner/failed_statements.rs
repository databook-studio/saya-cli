//! The run's statement-outcome memory: which SQL statements have failed (so a
//! byte-identical repeat is refused) and which succeeded last (so a budget
//! that runs out can still nominate an answer). The system prompt already
//! asks the model not to repeat a failed query; the model does not always obey,
//! so the failure set is a loop invariant, not advice.
//!
//! Statements are compared **exactly as submitted** — no whitespace or case
//! normalisation. A model that changes the SQL at all has changed its approach;
//! treating a genuine edit as a repeat would be far worse than missing one. A
//! statement that *succeeded* is never tracked as a failure: re-running a
//! successful query is legitimate (the model may re-confirm a result); only its
//! most recent success is remembered, as the salvage nomination candidate.

use crate::ToolCall;

/// Cap on the number of remembered failed statements. A long, degenerate run
/// cannot grow this without limit: once full, the oldest remembered failure is
/// dropped before a new one is inserted (FIFO). The benchmark pathology was one
/// statement repeated hundreds of times — a recent repeat is always caught, so
/// the cap bounds only how far back the memory reaches, not whether the common
/// case is caught. Most runs never approach it: a model that changes approach
/// produces distinct statements, and a normal run that never repeats a failure
/// records nothing here at all.
pub(super) const MAX_REMEMBERED_FAILURES: usize = 64;

/// The set of SQL statements that have failed during this run, bounded at
/// [`MAX_REMEMBERED_FAILURES`]. Keyed by the statement exactly as submitted;
/// the value is the error the last attempt produced, so a refusal can name it.
#[derive(Debug, Default)]
pub(super) struct FailedStatements {
    entries: Vec<(String, String)>,
}

impl FailedStatements {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The error a prior identical statement produced, if this exact SQL was
    /// tried and failed earlier in this run. `None` for a statement that never
    /// failed (or whose failure was evicted by the cap).
    pub(super) fn prior_error(&self, sql: &str) -> Option<&str> {
        self.entries
            .iter()
            .find_map(|(statement, error)| (statement == sql).then_some(error.as_str()))
    }

    /// Records that `sql` failed with `error`. Re-recording a statement already
    /// remembered updates its error in place (the latest failure is the useful
    /// one) without growing the set; a new statement, once the set is at
    /// [`MAX_REMEMBERED_FAILURES`], evicts the oldest remembered failure before
    /// inserting, so memory is bounded for a long run.
    pub(super) fn record(&mut self, sql: &str, error: &str) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|(statement, _)| statement == sql)
        {
            entry.1 = error.to_owned();
            return;
        }
        if self.entries.len() >= MAX_REMEMBERED_FAILURES {
            self.entries.remove(0);
        }
        self.entries.push((sql.to_owned(), error.to_owned()));
    }
}

/// The SQL statement a tool call carries, when its arguments include a `sql`
/// string. Used to track statements across a run — both failed statements (to
/// refuse a byte-identical repeat) and successful ones (to nominate the last
/// successful statement when a budget runs out). Returns the value as
/// submitted; **no normalisation** — a model that edits the SQL has changed its
/// approach. `None` for a call with no `sql` argument (e.g. `schema_discovery`)
/// or a non-string `sql`: nothing to track.
pub(super) fn sql_of(call: &ToolCall) -> Option<&str> {
    call.arguments
        .get("sql")
        .and_then(serde_json::Value::as_str)
}

/// Whether `call` is a byte-identical repeat of a statement that already failed
/// in this run. A call with no `sql` argument is never a repeat (there is no
/// statement to track); a call whose `sql` matches a remembered failure is.
pub(super) fn is_repeat(failed: &FailedStatements, call: &ToolCall) -> bool {
    sql_of(call)
        .and_then(|sql| failed.prior_error(sql).map(|_| true))
        .unwrap_or(false)
}

/// The completion summary for a refused repeat, kept as a constant so the
/// batch-path and sequential-path status derivations (`contains("failed")`)
/// agree.
pub(super) const REFUSAL_SUMMARY: &str = "statement already failed earlier in this run";

/// The tool result and summary returned when a repeat of a known failure is
/// refused. The result names the error the earlier attempt produced so the
/// model has the information it needs to change approach; the summary drives
/// `tool_metadata.status` (it contains "failed", so the status is "failed").
pub(super) fn refuse_repeat(prior_error: &str) -> (serde_json::Value, &'static str) {
    (
        serde_json::json!({"error": format!("statement already failed earlier in this run; it will fail again. Change your approach instead. Last error: {prior_error}")}),
        REFUSAL_SUMMARY,
    )
}

/// Records the outcome of an executed tool call into the run's failure memory
/// and success tracker. A failed statement is remembered (so a byte-identical
/// repeat is refused later) with the error it produced; a successful statement
/// becomes the nomination candidate when a budget later runs out. Calls that
/// were denied (not executed) and calls without a `sql` argument are no-ops —
/// a denial is not an execution, and only SQL statements are tracked.
pub(super) fn record_outcome(
    failed: &mut FailedStatements,
    last_successful_sql: &mut Option<String>,
    sql: Option<&str>,
    result: &serde_json::Value,
    executed: bool,
    summary: &str,
) {
    let Some(sql) = sql else {
        return;
    };
    if !executed {
        return;
    }
    if summary.contains("failed") {
        let error = result
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        failed.record(sql, error);
    } else {
        // The last statement that completed successfully is the best
        // available nomination when a budget runs out — most recent wins.
        *last_successful_sql = Some(sql.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prior_error_is_none_for_an_unseen_statement() {
        let set = FailedStatements::new();
        assert!(set.prior_error("SELECT 1").is_none());
    }

    #[test]
    fn record_then_prior_error_names_the_failure() {
        let mut set = FailedStatements::new();
        set.record("SELECT 1", "syntax error near '1'");
        assert_eq!(set.prior_error("SELECT 1"), Some("syntax error near '1'"));
    }

    #[test]
    fn a_statement_differing_by_one_character_is_not_a_repeat() {
        let mut set = FailedStatements::new();
        set.record("SELECT 1", "bad");
        // No normalisation: a one-character edit is a different statement.
        assert!(set.prior_error("SELECT 2").is_none());
    }

    #[test]
    fn recording_the_same_statement_twice_keeps_the_latest_error() {
        let mut set = FailedStatements::new();
        set.record("SELECT 1", "first error");
        set.record("SELECT 1", "second error");
        // The most recent record wins so the refusal names the latest failure.
        assert_eq!(set.prior_error("SELECT 1"), Some("second error"));
    }

    /// The set is bounded at `MAX_REMEMBERED_FAILURES`: after recording the
    /// cap's worth of distinct statements, the next record evicts the oldest,
    /// so a repeat of the evicted statement is no longer refused. The cap is
    /// the documented bound on memory for a long, degenerate run.
    #[test]
    fn the_remembered_failure_set_is_bounded_at_its_documented_cap() {
        let mut set = FailedStatements::new();
        for index in 0..MAX_REMEMBERED_FAILURES {
            set.record(&format!("SELECT {index}"), &format!("err {index}"));
        }
        assert_eq!(
            set.prior_error("SELECT 0"),
            Some("err 0"),
            "before overflow the oldest entry is still remembered"
        );
        // One more evicts the oldest ("SELECT 0").
        set.record("SELECT overflow", "err overflow");
        assert_eq!(
            set.prior_error("SELECT 0"),
            None,
            "the oldest entry is evicted once the cap is exceeded"
        );
        assert_eq!(
            set.prior_error("SELECT overflow"),
            Some("err overflow"),
            "the newest entry is remembered after the eviction"
        );
        // A recent entry that was not evicted is still refused.
        let recent = format!("SELECT {}", MAX_REMEMBERED_FAILURES - 1);
        let recent_err = format!("err {}", MAX_REMEMBERED_FAILURES - 1);
        assert_eq!(
            set.prior_error(&recent),
            Some(recent_err.as_str()),
            "a recent entry survives the eviction"
        );
    }

    #[test]
    fn sql_of_extracts_a_string_sql_argument() {
        let call = ToolCall {
            id: "c1".into(),
            name: "bounded_sql_query".into(),
            arguments: serde_json::json!({"sql": "SELECT 1"}),
        };
        assert_eq!(sql_of(&call), Some("SELECT 1"));
    }

    #[test]
    fn sql_of_is_none_when_there_is_no_sql_argument() {
        let call = ToolCall {
            id: "c1".into(),
            name: "schema_discovery".into(),
            arguments: serde_json::json!({}),
        };
        assert!(sql_of(&call).is_none());
    }

    #[test]
    fn sql_of_is_none_for_a_non_string_sql_argument() {
        let call = ToolCall {
            id: "c1".into(),
            name: "bounded_sql_query".into(),
            arguments: serde_json::json!({"sql": 42}),
        };
        assert!(sql_of(&call).is_none());
    }
}
