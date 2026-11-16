//! The task envelope for a background SQL run: what the command asked for,
//! the query the worker will run, and the follow-up that applies the
//! result when it lands.

use crate::interactive::sql_execution::SqlExecution;
use crate::slash::ExportRequest;
use std::sync::Arc;
use std::sync::Mutex;

/// What to do with a finished query result, captured when the command ran.
#[derive(Clone)]
pub(crate) enum Followup {
    /// Render the table and remember the query for /export & friends.
    Sql { connection: Option<String> },
    /// Write the result to a CSV/JSON path. Only the refresh forms of
    /// `/export` reach here (legacy and `--refresh`); `--snapshot` never
    /// dispatches a query. `mode: None` is the legacy form, whose success
    /// message may point at `--snapshot`.
    Export { request: ExportRequest },
    /// Build an HTML chart (optionally forced kind + output path).
    Chart {
        kind: Option<crate::chart::ChartKind>,
        path: Option<String>,
    },
    /// Render the plan table.
    Explain,
}

#[derive(Clone)]
pub(crate) struct SqlTask {
    pub profile: Option<String>,
    pub sql: String,
    pub followup: Followup,
    /// The envelope the worker's query ACTUALLY ran with, written by the
    /// worker before its `QueryResult` event. Per task, dropped with it:
    /// no global state, no keys, no eviction. A poisoned lock reads as
    /// "no envelope".
    pub execution: Arc<Mutex<Option<SqlExecution>>>,
}

impl SqlTask {
    pub(crate) fn new(profile: Option<String>, sql: String, followup: Followup) -> Self {
        Self {
            profile,
            sql,
            followup,
            execution: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn take_execution(&self) -> Option<SqlExecution> {
        self.execution.lock().ok()?.take()
    }

    /// Test helper: plants a synthetic envelope built from a `QueryResult`
    /// with an explicit source binding, so `complete` tests keep testing
    /// what they test without a live worker.
    #[cfg(test)]
    pub(crate) fn plant_execution_for_test(
        &self,
        result: &saya_types::QueryResult,
        binding: crate::interactive::sql_execution::SqlSourceBinding,
    ) {
        if let Ok(mut guard) = self.execution.lock() {
            let now = super::super::capture::unix_now_ms();
            *guard = Some(SqlExecution::new(result.clone(), binding, now, now));
        }
    }
}
