//! Blocking helpers that execute a command and RETURN a renderable event,
//! instead of printing to stdout (which would corrupt the alternate screen).
//! The TUI renders the returned event into the transcript.

use crate::config::runtime::RuntimeConfig;
use crate::interactive::sql_execution::SqlExecution;
use crate::interactive::sql_operation;
use crate::render::TerminalEvent;
use saya_agent::CancellationToken;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Bounded stash of executed-source envelopes, keyed by (profile, sql,
/// dispatch time). The worker stores the envelope with the row cap, identity,
/// dialect, and timestamps the query ACTUALLY used; the completion consumes
/// it so evidence never reads the mutable runtime. Bounded with FIFO
/// eviction so detached workers cannot grow it without limit.
fn envelope_stash() -> &'static Mutex<EnvelopeStash> {
    static STASH: OnceLock<Mutex<EnvelopeStash>> = OnceLock::new();
    STASH.get_or_init(|| Mutex::new(EnvelopeStash::default()))
}

#[derive(Default)]
struct EnvelopeStash {
    entries: HashMap<(String, String, i64), SqlExecution>,
    order: Vec<(String, String, i64)>,
}

/// Max stashed envelopes: the worker cap is 4, so 16 leaves headroom for
/// detached late completions without unbounded growth.
const MAX_STASHED_ENVELOPES: usize = 16;

pub(crate) fn stash_key(
    profile: Option<&str>,
    sql: &str,
    dispatched_unix_ms: i64,
) -> (String, String, i64) {
    (
        profile.unwrap_or("").to_owned(),
        sql.to_owned(),
        dispatched_unix_ms,
    )
}

pub(crate) fn stash_envelope(key: (String, String, i64), execution: &SqlExecution) {
    let stash = envelope_stash();
    let Ok(mut guard) = stash.lock() else { return };
    if guard
        .entries
        .insert(key.clone(), execution.clone())
        .is_none()
    {
        guard.order.push(key);
    }
    while guard.order.len() > MAX_STASHED_ENVELOPES {
        let oldest = guard.order.remove(0);
        guard.entries.remove(&oldest);
    }
}

pub(crate) fn take_stashed_envelope(key: &(String, String, i64)) -> Option<SqlExecution> {
    let stash = envelope_stash();
    let Ok(mut guard) = stash.lock() else {
        return None;
    };
    let execution = guard.entries.remove(key)?;
    guard.order.retain(|k| k != key);
    Some(execution)
}

/// Runs a raw SQL query without prompting (the TUI owns the screen).
/// Async for parity with the spawned worker: the body is the executed source
/// envelope (binding + timestamps), stashed under the task's dispatch key so
/// the completion builds evidence from the EXECUTED source. The dispatch key
/// rides `SqlTask.started_unix_ms` (unix millis at dispatch); spawning and
/// completion must use the same task value. Errors stash nothing.
pub(crate) async fn run_sql_with_key(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    cancellation: &CancellationToken,
    dispatched_unix_ms: i64,
) -> TerminalEvent {
    match sql_operation::execute_with_cancellation_and_source(
        runtime,
        profile_name,
        sql,
        false,
        cancellation,
    )
    .await
    {
        Ok(execution) => {
            let event = TerminalEvent::QueryResult {
                result: execution.result.clone(),
            };
            stash_envelope(stash_key(profile_name, sql, dispatched_unix_ms), &execution);
            event
        }
        Err(error) => TerminalEvent::Error {
            message: error.to_string(),
        },
    }
}
