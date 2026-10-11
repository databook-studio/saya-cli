//! Outcome of dispatching one line: the caller owns the follow-up.

/// Outcome of dispatching one line.
pub(crate) enum Dispatch {
    Coding(crate::cli::CodingCommand),
    /// A command was handled synchronously; keep looping.
    Handled,
    /// The session should exit.
    Quit,
    /// The line is a prompt for the agent; the caller starts streaming it.
    Agent(String),
    /// Open the interactive session picker.
    OpenSessionPicker,
    /// Open the interactive saved-investigation picker (bare `/investigations`).
    OpenInvestigationPicker,
    /// A SQL-backed command runs on a worker thread; the caller stores the
    /// receiver and applies [`sql_task::complete`] when it finishes.
    SqlTask(super::super::sql_task::SqlTask),
    /// `/investigation run` — the replay runs on a worker thread like a SQL
    /// command; the caller stores the receiver and applies
    /// [`replay_task::complete`] when it finishes (D13).
    ReplayTask(super::super::replay_task::ReplayTask),
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
    /// `/setup` — the guided-setup handoff: the caller suspends the TUI's
    /// terminal modes, runs the wizard in the normal screen, and renders the
    /// outcome into the transcript. Never dispatched while the app is busy —
    /// the queued-prompt gate holds it like any other command.
    RunSetup,
    /// `/reload` — re-read the configuration with the launch options and
    /// swap the app's runtime handle between turns. The caller (the loop's
    /// queued-prompt drain) owns the swap: it holds the launch options and
    /// the app, and a failed reload changes nothing.
    Reload,
}
