//! Outcome of dispatching one line: the caller owns the follow-up.

/// Outcome of dispatching one line.
pub(crate) enum Dispatch {
    /// A command was handled synchronously; keep looping.
    Handled,
    /// The session should exit.
    Quit,
    /// The line is a prompt for the agent; the caller starts streaming it.
    Agent(String),
    /// Open the interactive session picker.
    OpenSessionPicker,
    /// A SQL-backed command runs on a worker thread; the caller stores the
    /// receiver and applies [`sql_task::complete`] when it finishes.
    SqlTask(super::super::sql_task::SqlTask),
    /// `/columns` — set which columns wide result tables show. Handled by the
    /// caller, which owns the view state on `App`.
    SetColumns(Option<String>),
    /// `/compact` — shrink working memory through a bounded summariser call.
    /// Runs on a worker task like a SQL command; the caller owns the handle.
    Compact,
    /// `/run <goal…>` — a fresh run the run panel drives as a worker task;
    /// the caller owns the panel state and the worker handle.
    RunPanel {
        goal: Option<String>,
        allow: Vec<String>,
        budget: Vec<String>,
    },
}
