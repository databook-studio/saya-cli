//! The run's call-outcome memory: which tool calls have failed (so a
//! byte-identical repeat is refused) and which SQL statement succeeded last
//! (so a budget that runs out can still nominate an answer). The system
//! prompt already asks the model not to repeat a failed query; the model
//! does not always obey, so the failure set is a loop invariant, not advice.
//!
//! Failures are compared **exactly as submitted** — a SQL statement with no
//! whitespace or case normalisation, any other tool with no argument
//! coercion. A model that changes either at all has changed its approach;
//! treating a genuine edit as a repeat would be far worse than missing one.
//! (Canonicalising the *serialization* of a non-SQL call's arguments in
//! `failure_key.rs` is not an exception: object key order is a construction
//! detail of the value, not part of the model's approach, so two calls
//! carrying the same keys in a different order are the same call.) A call
//! that *succeeded* is never tracked as a failure: re-running it is
//! legitimate (the model may re-confirm a result); only the most recent
//! success is remembered, as the salvage nomination candidate — and only for
//! SQL, the medium the nomination answers from.

use super::failure_key::{CallKey, key_of};
use crate::ToolCall;

/// Cap on the number of remembered failed calls. A long, degenerate run
/// cannot grow this without limit: once full, the oldest remembered failure
/// is dropped before a new one is inserted (FIFO). The benchmark pathology
/// was one statement repeated hundreds of times — a recent repeat is always
/// caught, so the cap bounds only how far back the memory reaches, not
/// whether the common case is caught. Most runs never approach it: a model
/// that changes approach produces distinct calls, and a normal run that
/// never repeats a failure records nothing here at all.
pub(super) const MAX_REMEMBERED_FAILURES: usize = 64;
const MAX_REMEMBERED_FAILURE_BYTES: usize = 4096;

/// The set of calls that have failed during this run, bounded at
/// [`MAX_REMEMBERED_FAILURES`]. Keyed by [`CallKey`] — the SQL statement
/// exactly as submitted for a call carrying `sql`, the tool name plus
/// canonically-serialized arguments for every other tool (`failure_key.rs`);
/// the value is the error the last attempt produced, so a refusal can name
/// it.
#[derive(Debug, Default)]
pub(super) struct FailedStatements {
    entries: Vec<(CallKey, String)>,
}

impl FailedStatements {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The error a prior identical call produced, if this exact call was
    /// tried and failed earlier in this run. `None` for a call that never
    /// failed (or whose failure was evicted by the cap).
    pub(super) fn prior_error(&self, key: &CallKey) -> Option<&str> {
        self.entries
            .iter()
            .find_map(|(entry, error)| (entry == key).then_some(error.as_str()))
    }

    /// Records that `key` failed with `error`. Re-recording a call already
    /// remembered updates its error in place (the latest failure is the
    /// useful one) without growing the set; a new call, once the set is at
    /// [`MAX_REMEMBERED_FAILURES`], evicts the oldest remembered failure
    /// before inserting, so memory is bounded for a long run.
    pub(super) fn record(&mut self, key: CallKey, error: &str) {
        let error = bound_error(error);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|(existing, _)| existing == &key)
        {
            entry.1 = error;
            return;
        }
        if self.entries.len() >= MAX_REMEMBERED_FAILURES {
            self.entries.remove(0);
        }
        self.entries.push((key, error));
    }
}

pub(super) fn bound_error(error: &str) -> String {
    if error.len() <= MAX_REMEMBERED_FAILURE_BYTES {
        return error.to_owned();
    }
    let marker = "...[truncated]";
    let head = super::tools::floor_boundary(
        error,
        MAX_REMEMBERED_FAILURE_BYTES.saturating_sub(marker.len()),
    );
    format!("{}{marker}", &error[..head])
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

/// Whether `call` is a byte-identical repeat of a call that already failed
/// in this run: the same SQL statement for a call carrying `sql` (a different
/// `connection` argument does not make a known-bad statement look fresh), or
/// the same tool invocation — name and arguments — for every other tool.
pub(super) fn is_repeat(failed: &FailedStatements, call: &ToolCall) -> bool {
    failed.prior_error(&key_of(call)).is_some()
}

/// The completion summary for a refused repeat, kept as a constant so the
/// batch and sequential typed outcomes agree. The wording names the *call* —
/// statement or not — so it stays honest for a non-SQL tool.
pub(super) const REFUSAL_SUMMARY: &str = "call already failed earlier in this run";

/// The tool result and summary returned when a repeat of a known failure is
/// refused. The result names the error the earlier attempt produced so the
/// model has the information it needs to change approach; callers retain the
/// refusal's typed failed outcome alongside this model-facing text.
pub(super) fn refuse_repeat(prior_error: &str) -> (serde_json::Value, &'static str) {
    (
        serde_json::json!({"error": format!("this call already failed earlier in this run; it will fail again. Change your approach instead. Last error: {prior_error}")}),
        REFUSAL_SUMMARY,
    )
}

/// Records the outcome of an executed tool call into the run's failure memory
/// and success tracker. A failed call is remembered under its [`CallKey`] (so
/// a byte-identical repeat is refused later) with the error it produced; a
/// successful *statement* becomes the nomination candidate when a budget
/// later runs out, while a successful non-SQL call nominates nothing — the
/// salvage answer is SQL. Calls that were denied (not executed) are no-ops: a
/// denial is not an execution.
pub(super) fn record_outcome(
    failed: &mut FailedStatements,
    last_successful_sql: &mut Option<String>,
    key: CallKey,
    executed: bool,
    succeeded: bool,
    failure_reason: Option<&str>,
) {
    if !executed {
        return;
    }
    if !succeeded {
        failed.record(key, failure_reason.unwrap_or_default());
    } else if let CallKey::Sql(sql) = key {
        // The last statement that completed successfully is the best
        // available nomination when a budget runs out — most recent wins.
        *last_successful_sql = Some(sql);
    }
}

#[cfg(test)]
#[path = "failed_statements_tests.rs"]
mod tests;
