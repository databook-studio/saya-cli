use std::{
    fmt,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use saya_agent::CancellationToken;
use thiserror::Error;
use tokio::{sync::OwnedSemaphorePermit, task::JoinHandle};

use crate::runner::{ProgramOutcome, RunProgram, RunnerError};

/// A supervised `RunProgram` call. Dropping the handle requests cancellation;
/// the task remains alive until the existing runner has cleaned up the process.
pub struct CommandSession {
    task: JoinHandle<Result<ProgramOutcome, CommandSessionError>>,
    cancellation: CancelOnDrop,
}

impl fmt::Debug for CommandSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandSession")
            .field("is_finished", &self.is_finished())
            .finish()
    }
}

impl CommandSession {
    pub(super) fn spawn(
        runner: RunProgram,
        program: String,
        argv: Vec<String>,
        timeout_seconds: NonZeroU64,
        owner_cancellation: CancellationToken,
        permit: OwnedSemaphorePermit,
    ) -> Self {
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let task = tokio::spawn(async move {
            let _permit = permit;
            if task_cancellation.is_cancelled() || owner_cancellation.is_cancelled() {
                task_cancellation.cancel();
                return Err(CommandSessionError::CancelledBeforeStart);
            }
            let runner = runner.with_cancellation(task_cancellation.clone());
            let run_started = Arc::new(AtomicBool::new(false));
            let mark_started = Arc::clone(&run_started);
            let run = async move {
                mark_started.store(true, Ordering::Release);
                runner
                    .run(&program, &argv, Some(timeout_seconds.get()))
                    .await
            };
            tokio::pin!(run);
            let completed = tokio::select! {
                biased;
                _ = task_cancellation.cancelled() => None,
                _ = owner_cancellation.cancelled() => None,
                result = &mut run => Some(result),
            };
            match completed {
                Some(result) => result.map_err(CommandSessionError::Runner),
                None if run_started.load(Ordering::Acquire) => {
                    task_cancellation.cancel();
                    run.await.map_err(CommandSessionError::Runner)
                }
                None => {
                    task_cancellation.cancel();
                    Err(CommandSessionError::CancelledBeforeStart)
                }
            }
        });
        Self {
            task,
            cancellation: CancelOnDrop(cancellation),
        }
    }

    /// Reports only whether the owned supervision future has finished. It does
    /// not claim that an operating-system process has started or is running.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Requests this session to stop without releasing its owner's capacity.
    /// Call `join` or `stop_and_join` to observe cleanup completion.
    pub fn stop(&self) {
        self.cancellation.0.cancel();
    }

    /// Waits for the original `RunProgram` outcome or its cause-preserving error.
    pub async fn join(self) -> Result<ProgramOutcome, CommandSessionError> {
        join_task(self.task).await
    }

    /// Stops this command, then waits until the existing runner reports after
    /// process cleanup and reaping.
    pub async fn stop_and_join(self) -> Result<ProgramOutcome, CommandSessionError> {
        self.cancellation.0.cancel();
        join_task(self.task).await
    }
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn join_task(
    task: JoinHandle<Result<ProgramOutcome, CommandSessionError>>,
) -> Result<ProgramOutcome, CommandSessionError> {
    match task.await {
        Ok(result) => result,
        Err(source) => Err(CommandSessionError::Join(source)),
    }
}

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
}
