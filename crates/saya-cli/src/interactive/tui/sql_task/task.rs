//! The task envelope for a background SQL run: which profile and query the
//! worker will run, and the slot the worker writes its real execution envelope
//! into. The old surface's "follow-up" (render the table, export it, chart it)
//! went with it in X1: the new surface files the result in its own preview.

use crate::interactive::sql_execution::SqlExecution;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Clone)]
pub(crate) struct SqlTask {
    pub profile: Option<String>,
    pub sql: String,
    /// The envelope the worker's query ACTUALLY ran with, written by the
    /// worker before its `QueryResult` event. Per task, dropped with it:
    /// no global state, no keys, no eviction. A poisoned lock reads as
    /// "no envelope".
    pub execution: Arc<Mutex<Option<SqlExecution>>>,
}

impl SqlTask {
    pub(crate) fn new(profile: Option<String>, sql: String) -> Self {
        Self {
            profile,
            sql,
            execution: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn take_execution(&self) -> Option<SqlExecution> {
        self.execution.lock().ok()?.take()
    }
}
