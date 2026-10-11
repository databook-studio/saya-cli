use thiserror::Error;
use tokio::sync::watch;

use crate::runner::{CommandProcessState, ProgramOutcome, RunnerError, stdin::StdinDelivery};

/// Why a supervised command could not produce its existing runner outcome.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CommandSessionError {
    #[error(transparent)]
    Runner(#[from] RunnerError),
    #[error("command supervision task failed to join")]
    Join(#[source] tokio::task::JoinError),
    #[error("command owner stopped before runner dispatch")]
    CancelledBeforeStart,
    #[error("command owner stopped a session whose runner refused before any process existed")]
    StoppedBeforeSpawn {
        #[source]
        source: RunnerError,
    },
    #[error(
        "command exited after only {written_bytes} of {accepted_bytes} queued stdin bytes reached its pipe"
    )]
    InputIncomplete {
        outcome: Box<ProgramOutcome>,
        accepted_bytes: usize,
        written_bytes: usize,
    },
    #[error(
        "command stdin write failed after {written_bytes} of {accepted_bytes} queued bytes reached its pipe: {source}"
    )]
    InputWriteFailed {
        outcome: Box<ProgramOutcome>,
        accepted_bytes: usize,
        written_bytes: usize,
        #[source]
        source: std::io::Error,
    },
}

pub(super) fn input_result(
    outcome: ProgramOutcome,
    stdin: Option<StdinDelivery>,
) -> Result<ProgramOutcome, CommandSessionError> {
    if outcome.cancelled || outcome.killed_by_timeout {
        return Ok(outcome);
    }
    let Some(stdin) = stdin.filter(|delivery| !delivery.is_complete()) else {
        return Ok(outcome);
    };
    match stdin.source {
        Some(source) => Err(CommandSessionError::InputWriteFailed {
            outcome: Box::new(outcome),
            accepted_bytes: stdin.accepted_bytes,
            written_bytes: stdin.written_bytes,
            source,
        }),
        None => Err(CommandSessionError::InputIncomplete {
            outcome: Box::new(outcome),
            accepted_bytes: stdin.accepted_bytes,
            written_bytes: stdin.written_bytes,
        }),
    }
}

pub(super) fn mark_failed_process_state(state: &watch::Sender<CommandProcessState>) {
    state.send_modify(|current| {
        *current = match *current {
            CommandProcessState::Pending => CommandProcessState::NotSpawned,
            CommandProcessState::Spawned { pid } => CommandProcessState::Unknown { pid },
            current @ (CommandProcessState::Reaped { .. }
            | CommandProcessState::NotSpawned
            | CommandProcessState::Unknown { .. }) => current,
        };
    });
}
