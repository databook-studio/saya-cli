//! The run's call-outcome memory: how many times each tool call has failed
//! (so a call that keeps failing the same way can be told to change approach)
//! and which SQL statement succeeded last (so a budget that runs out can still
//! nominate an answer). The system prompt already asks the model not to repeat
//! a failed query; the model does not always obey, and repeating it is how it
//! learns the call is hopeless — so the memory never blocks a call, it only
//! counts.
//!
//! Failures are compared **exactly as submitted** — a SQL statement with no
//! whitespace or case normalisation, any other tool with no argument
//! coercion. A model that changes either at all has changed its approach, and
//! a genuine edit must never be counted as a repeat of an earlier call.
//! (Canonicalising the *serialization* of a non-SQL call's arguments in
//! `failure_key.rs` is not an exception: object key order is a construction
//! detail of the value, not part of the model's approach, so two calls
//! carrying the same keys in a different order are the same call.) A call
//! that *succeeded* is never tracked as a failure: re-running it is
//! legitimate (the model may re-confirm a result); only the most recent
//! success is remembered, as the salvage nomination candidate — and only for
//! SQL, the medium the nomination answers from.

use super::failure_key::CallKey;
use crate::ToolCall;

/// Cap on the number of remembered failed calls. A long, degenerate run
/// cannot grow this without limit: once full, the oldest remembered failure
/// is dropped before a new one is inserted (FIFO). The observed pathology
/// was one statement repeated hundreds of times — a recent repeat is always
/// caught, so the cap bounds only how far back the memory reaches, not
/// whether the common case is caught. Most runs never approach it: a model
/// that changes approach produces distinct calls, and a normal run that
/// never repeats a failure records nothing here at all.
pub(super) const MAX_REMEMBERED_FAILURES: usize = 64;

/// How many identical failures of one call earn the note. The third failure
/// is the point at which repeating the call has plainly stopped being a
/// search and become a loop.
const REPEATS_BEFORE_NOTE: u32 = 3;

/// The note appended to a failed result once this exact call has failed
/// [`REPEATS_BEFORE_NOTE`] times in one run. It is offered once per call per
/// run — never on the fourth failure, and never again — so a model that keeps
/// repeating hears the advice exactly when it is due and not as a nag.
pub(super) const REPEAT_NOTE: &str =
    "This exact call has now failed 3 times. Try a different approach.";

/// The set of calls that have failed during this run, bounded at
/// [`MAX_REMEMBERED_FAILURES`]. Keyed by [`CallKey`] — the SQL statement
/// exactly as submitted for a call carrying `sql`, the tool name plus
/// canonically-serialized arguments for every other tool (`failure_key.rs`);
/// the value is how many times that exact call has failed so far.
#[derive(Debug, Default)]
pub(super) struct FailedStatements {
    entries: Vec<(CallKey, u32)>,
}

impl FailedStatements {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// How many times this exact call has failed in this run. `0` for a call
    /// that never failed (or whose failures were evicted by the cap).
    #[cfg(test)]
    pub(super) fn failures(&self, key: &CallKey) -> u32 {
        self.entries
            .iter()
            .find_map(|(entry, count)| (entry == key).then_some(*count))
            .unwrap_or(0)
    }

    pub(super) fn forget(&mut self, key: &CallKey) {
        if let Some(index) = self.entries.iter().position(|(entry, _)| entry == key) {
            self.entries.remove(index);
        }
    }

    /// Records that `key` failed once more. Re-recording a call already
    /// remembered advances its count in place without growing the set; a new
    /// call, once the set is at [`MAX_REMEMBERED_FAILURES`], evicts the oldest
    /// remembered failure before inserting, so memory is bounded for a long
    /// run. Returns [`REPEAT_NOTE`] exactly when this failure is the third of
    /// its key, and `None` otherwise.
    fn record_failure(&mut self, key: CallKey) -> Option<&'static str> {
        if let Some((_, count)) = self
            .entries
            .iter_mut()
            .find(|(existing, _)| existing == &key)
        {
            *count = count.saturating_add(1);
            return (*count == REPEATS_BEFORE_NOTE).then_some(REPEAT_NOTE);
        }
        if self.entries.len() >= MAX_REMEMBERED_FAILURES {
            self.entries.remove(0);
        }
        self.entries.push((key, 1));
        None
    }
}

/// The SQL statement a tool call carries, when its arguments include a `sql`
/// string. Used to track statements across a run — both failed statements (so
/// a repeat is recognised) and successful ones (to nominate the last
/// successful statement when a budget runs out). Returns the value as
/// submitted; **no normalisation** — a model that edits the SQL has changed its
/// approach. `None` for a call with no `sql` argument (e.g. `schema_discovery`)
/// or a non-string `sql`: nothing to track.
pub(super) fn sql_of(call: &ToolCall) -> Option<&str> {
    call.arguments
        .get("sql")
        .and_then(serde_json::Value::as_str)
}

/// Records the outcome of an executed tool call into the run's failure memory
/// and success tracker. A failed call advances its [`CallKey`]'s count; a
/// successful *statement* becomes the nomination candidate when a budget
/// later runs out, while a successful non-SQL call nominates nothing — the
/// salvage answer is SQL. Calls that were denied (not executed) are no-ops: a
/// denial is not an execution. Returns [`REPEAT_NOTE`] when this failure is
/// the third identical one, so the caller can append it to the failed result.
pub(super) fn record_outcome(
    failed: &mut FailedStatements,
    last_successful_sql: &mut Option<String>,
    key: CallKey,
    executed: bool,
    succeeded: bool,
) -> Option<&'static str> {
    if !executed {
        return None;
    }
    if !succeeded {
        return failed.record_failure(key);
    }
    failed.forget(&key);
    if let CallKey::Sql(sql) = key {
        // The last statement that completed successfully is the best
        // available nomination when a budget runs out — most recent wins.
        *last_successful_sql = Some(sql);
    }
    None
}

/// Appends the repeat note to a failed call's result, so the model reads it as
/// part of the failure the call produced. It follows the error text, exactly as
/// the refusal guidance does: a result with no `error` string
/// carries the note under its own key instead, and a result that is not an
/// object carries nothing.
pub(super) fn append_note(result: &mut serde_json::Value, note: &str) {
    let serde_json::Value::Object(fields) = result else {
        return;
    };
    match fields.get_mut("error") {
        Some(serde_json::Value::String(error)) => {
            error.push_str("\n\n");
            error.push_str(note);
        }
        _ => {
            fields.insert("note".into(), serde_json::Value::String(note.into()));
        }
    }
}

#[cfg(test)]
#[path = "failed_statements_tests.rs"]
mod tests;
