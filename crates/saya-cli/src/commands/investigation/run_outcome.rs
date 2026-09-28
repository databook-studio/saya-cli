//! The typed outcome of a saved-investigation replay (D12, C3): the shared
//! `investigation run` operation returns what happened — the exit code the
//! CLI adapter returns and, after a successful execution, the replay itself
//! — so adapters print or capture from this value instead of re-parsing
//! rendered output.

use saya_types::{ExecutionEvidence, QueryResult};

/// What one replay produced. The replay is Some only after a successful
/// execution — including when a later step (the `--report` write) refused —
/// and None for every refusal and execution failure, so a caller can never
/// capture a result that did not come from this run.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub code: i32,
    pub replay: Option<Replay>,
}

/// One successful execution of the saved SQL: the connector's result, its
/// evidence, the exact SQL that ran, and the connection it ran on — what a
/// capture needs to bind a snapshot or refresh (D12).
#[derive(Debug, Clone)]
pub struct Replay {
    pub result: QueryResult,
    pub evidence: ExecutionEvidence,
    pub sql: String,
    pub connection: String,
}

impl RunOutcome {
    /// The outcome for a terminal code with no replay: every refusal,
    /// execution failure, and non-run investigation command.
    pub(crate) fn plain(code: i32) -> Self {
        Self { code, replay: None }
    }
}
