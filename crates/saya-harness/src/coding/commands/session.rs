use std::{
    fmt,
    num::NonZeroU64,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use saya_agent::CancellationToken;
use tokio::{
    sync::{OwnedSemaphorePermit, watch},
    task::JoinHandle,
};

use crate::runner::{
    CommandProcessState, ProgramOutcome, RunProgram, stdin_channel::StdinInputPipe,
};

use super::cursor::{CommandOutputCursor, CommandOutputRead};
use super::input::{CommandInput, CommandInputError, StdinQueueReceipt};
use super::output::{CommandOutputError, CommandOutputStatus, CommandOutputStore};
#[path = "session_errors.rs"]
mod errors;
pub use errors::CommandSessionError;
use errors::{input_result, mark_failed_process_state};

/// A supervised `RunProgram` call. Dropping the handle requests cancellation;
/// the task remains alive until the existing runner has cleaned up the process.
pub struct CommandSession {
    task: JoinHandle<Result<ProgramOutcome, CommandSessionError>>,
    cancellation: CancelOnDrop,
    input: Option<CommandInput>,
    process_state: watch::Receiver<CommandProcessState>,
    output: Option<Arc<CommandOutputStore>>,
}

impl fmt::Debug for CommandSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandSession")
            .field("is_finished", &self.is_finished())
            .finish()
    }
}

/// The supervised run a session supervises: the already-authorized runner
/// call plus the program specification it will execute. Bundled so the
/// spawn seam stays a small, stable shape.
pub(super) struct SupervisedRun {
    pub(super) runner: RunProgram,
    pub(super) program: String,
    pub(super) argv: Vec<String>,
    pub(super) timeout_seconds: NonZeroU64,
}

impl CommandSession {
    pub(super) fn spawn(
        run: SupervisedRun,
        owner_cancellation: CancellationToken,
        permit: OwnedSemaphorePermit,
        input: Option<(CommandInput, StdinInputPipe)>,
        output: Option<Arc<CommandOutputStore>>,
    ) -> Self {
        let cancellation = CancellationToken::new();
        let task_cancellation = cancellation.clone();
        let (input, input_pipe) =
            input.map_or((None, None), |(input, pipe)| (Some(input), Some(pipe)));
        let output_sinks = output.as_ref().map(|store| store.sinks());
        let (process_state_tx, process_state) = watch::channel(CommandProcessState::Pending);
        let task = tokio::spawn(async move {
            let _permit = permit;
            let SupervisedRun {
                runner,
                program,
                argv,
                timeout_seconds,
            } = run;
            let run_cancellation = runner.cancellation_token();
            if task_cancellation.is_cancelled()
                || owner_cancellation.is_cancelled()
                || run_cancellation.is_cancelled()
            {
                task_cancellation.cancel();
                process_state_tx.send_replace(CommandProcessState::NotSpawned);
                return Err(CommandSessionError::CancelledBeforeStart);
            }
            let runner = runner.with_cancellation(task_cancellation.clone());
            let run_started = Arc::new(AtomicBool::new(false));
            let mark_started = Arc::clone(&run_started);
            let run_process_state = process_state_tx.clone();
            let run = async move {
                mark_started.store(true, Ordering::Release);
                runner
                    .run_command(
                        &program,
                        &argv,
                        Some(timeout_seconds.get()),
                        input_pipe,
                        run_process_state,
                        output_sinks,
                    )
                    .await
            };
            tokio::pin!(run);
            let completed = tokio::select! {
                biased;
                _ = task_cancellation.cancelled() => None,
                _ = owner_cancellation.cancelled() => None,
                _ = run_cancellation.cancelled() => None,
                result = &mut run => Some(result),
            };
            match completed {
                Some(Ok(run)) => input_result(run.outcome, run.stdin),
                Some(Err(error)) => {
                    mark_failed_process_state(&process_state_tx);
                    Err(CommandSessionError::Runner(error))
                }
                None if run_started.load(Ordering::Acquire) => {
                    task_cancellation.cancel();
                    match run.await {
                        Ok(run) => input_result(run.outcome, run.stdin),
                        Err(error) => {
                            mark_failed_process_state(&process_state_tx);
                            Err(CommandSessionError::Runner(error))
                        }
                    }
                }
                None => {
                    task_cancellation.cancel();
                    process_state_tx.send_replace(CommandProcessState::NotSpawned);
                    Err(CommandSessionError::CancelledBeforeStart)
                }
            }
        });
        Self {
            task,
            cancellation: CancelOnDrop(cancellation),
            input,
            process_state,
            output,
        }
    }

    /// Reports only whether the owned supervision future has finished. It does
    /// not claim that an operating-system process has started or is running.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Returns the last process lifecycle observed by the existing runner.
    /// Spawned identifies an OS child PID, not an independent liveness probe.
    pub fn process_state(&self) -> CommandProcessState {
        *self.process_state.borrow()
    }

    /// The nonce-named directory holding this session's stored command
    /// output, when the session was started with an output store. The
    /// per-stream files inside are named by `OutputStream::file_name`;
    /// bounded reads are `CommandSession::read_output`.
    pub fn output_dir(&self) -> Option<PathBuf> {
        self.output.as_ref().map(|store| store.dir().to_path_buf())
    }

    /// Serves at most `max_bytes` stored bytes from `cursor` — the bounded
    /// redacted output read (C02b). A session started without an output
    /// store is typed `Unavailable`; the store read path reports the rest
    /// (closed store, invalid cursor, oversized read, store failure) itself.
    pub fn read_output(
        &self,
        cursor: CommandOutputCursor,
        max_bytes: usize,
    ) -> Result<CommandOutputRead, CommandOutputError> {
        let store = self
            .output
            .as_ref()
            .ok_or(CommandOutputError::Unavailable)?;
        store.read(cursor, max_bytes)
    }

    /// The typed output-store accounting once the run's outcome is settled and
    /// both streams finished; `None` while the run is still producing output
    /// or the session was started without one. Call it before consuming the
    /// session through `join` or `stop_and_join`. A writer failure or a
    /// failed `sync_all` is never reported as a clean store.
    pub fn output_status(&self) -> Option<CommandOutputStatus> {
        let store = self.output.as_ref()?;
        store.output_status()
    }

    /// Captures complete raw redacted bytes from one settled private stream.
    /// Both writers must be clean and uncapped. Bytes come from the immutable
    /// writer snapshot, never the current file. The fixed byte cap is retained;
    /// the cursor read limit is unchanged and no filesystem path is opened.
    /// This does not certify process exit; call before consuming `join`.
    pub fn capture_output(
        &self,
        stream: super::OutputStream,
    ) -> Result<super::CommandOutputCapture, CommandOutputError> {
        self.output
            .as_ref()
            .ok_or(CommandOutputError::Unavailable)?
            .capture_complete(stream)
    }

    /// Queues one bounded stdin chunk. Success means queued, not written to
    /// the kernel pipe or consumed by the command.
    pub fn send_stdin(&self, bytes: &[u8]) -> Result<StdinQueueReceipt, CommandInputError> {
        self.input
            .as_ref()
            .ok_or(CommandInputError::Unavailable)?
            .send(bytes)
    }

    /// Closes stdin after already queued bytes drain. Later writes are refused.
    pub fn close_stdin(&self) -> Result<(), CommandInputError> {
        self.input
            .as_ref()
            .ok_or(CommandInputError::Unavailable)?
            .close()
    }

    /// Requests this session to stop without releasing its owner's capacity.
    /// Call `join` or `stop_and_join` to observe cleanup completion.
    pub fn stop(&self) {
        if let Some(input) = &self.input {
            input.close_if_open();
        }
        self.cancellation.0.cancel();
    }

    /// Waits for the original `RunProgram` outcome or its cause-preserving error.
    pub async fn join(self) -> Result<ProgramOutcome, CommandSessionError> {
        if let Some(input) = &self.input {
            input.close_if_open();
        }
        join_task(self.task).await
    }

    /// Stops this command, then waits until the existing runner reports after
    /// process cleanup and reaping.
    pub async fn stop_and_join(self) -> Result<ProgramOutcome, CommandSessionError> {
        if let Some(input) = &self.input {
            input.close_if_open();
        }
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
