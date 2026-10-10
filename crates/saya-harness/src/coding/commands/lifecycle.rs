use std::{
    fs,
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::Arc,
};

use saya_agent::CancellationToken;
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::runner::RunProgram;

use super::{CommandSession, input::CommandInput, output::CommandOutputStore};

pub const MAX_COMMAND_CONCURRENCY: usize = 2;

/// Owner for a finite number of existing, already-authorized RunProgram calls.
pub struct CommandOwner {
    cancellation: CancellationToken,
    active: Arc<Semaphore>,
}

impl CommandOwner {
    /// Creates a command owner with one or two concurrent runner slots.
    pub fn new(max_concurrent: usize) -> Result<Self, CommandOwnerError> {
        if !(1..=MAX_COMMAND_CONCURRENCY).contains(&max_concurrent) {
            return Err(CommandOwnerError::InvalidConcurrency);
        }
        Ok(Self {
            cancellation: CancellationToken::new(),
            active: Arc::new(Semaphore::new(max_concurrent)),
        })
    }

    /// Starts one owned runner call if capacity is available. Refusal happens
    /// before a supervision task or external process is created.
    pub fn start(
        &self,
        runner: RunProgram,
        program: String,
        argv: Vec<String>,
        timeout_seconds: NonZeroU64,
    ) -> Result<CommandSession, CommandStartError> {
        self.start_inner(runner, program, argv, timeout_seconds, false, None)
    }

    /// Starts a runner with a bounded post-spawn stdin queue and explicit EOF.
    /// This adds input control only; the caller-created RunProgram still
    /// supplies the allowlist, credentials, timeout default, and proven spawn.
    pub fn start_with_stdin(
        &self,
        runner: RunProgram,
        program: String,
        argv: Vec<String>,
        timeout_seconds: NonZeroU64,
    ) -> Result<CommandSession, CommandStartError> {
        self.start_inner(runner, program, argv, timeout_seconds, true, None)
    }

    /// Starts a runner call whose captured output is appended, redacted, to
    /// a private bounded store under `store_root`. The store is prepared
    /// before any task, permit, or process exists: a root that cannot be
    /// prepared refuses the start and creates nothing beyond its own root,
    /// and a failed preparation removes the nonce it may have created. A
    /// start refused after preparation — owner stopped or at capacity —
    /// leaves no store directory behind: the guard removes the nonce
    /// directory this attempt created.
    pub fn start_with_output(
        &self,
        runner: RunProgram,
        program: String,
        argv: Vec<String>,
        timeout_seconds: NonZeroU64,
        store_root: PathBuf,
    ) -> Result<CommandSession, CommandStartError> {
        let output = match CommandOutputStore::prepare(&store_root) {
            Ok(store) => Some(Arc::new(store)),
            Err(_) => return Err(CommandStartError::OutputStore),
        };
        let guard = output
            .as_deref()
            .map(|store| PreparedStoreGuard::arm(store, store_root));
        match self.start_inner(runner, program, argv, timeout_seconds, false, output) {
            Ok(session) => {
                if let Some(guard) = guard {
                    guard.defuse();
                }
                Ok(session)
            }
            // The guard drops here and removes only this attempt's nonce
            // directory.
            Err(error) => Err(error),
        }
    }

    fn start_inner(
        &self,
        runner: RunProgram,
        program: String,
        argv: Vec<String>,
        timeout_seconds: NonZeroU64,
        stdin_enabled: bool,
        output: Option<Arc<CommandOutputStore>>,
    ) -> Result<CommandSession, CommandStartError> {
        if self.cancellation.is_cancelled() {
            return Err(CommandStartError::OwnerStopped);
        }
        let permit = Arc::clone(&self.active)
            .try_acquire_owned()
            .map_err(|_| CommandStartError::AtCapacity)?;
        if self.cancellation.is_cancelled() {
            return Err(CommandStartError::OwnerStopped);
        }
        let input = stdin_enabled.then(|| CommandInput::channel(self.cancellation.clone()));
        Ok(CommandSession::spawn(
            runner,
            program,
            argv,
            timeout_seconds,
            self.cancellation.clone(),
            permit,
            input,
            output,
        ))
    }

    /// Cancels every session started by this owner. Each session still owns
    /// its own token and must be joined before the caller reports completion.
    pub fn cancel_all(&self) {
        self.cancellation.cancel();
    }
}

impl Drop for CommandOwner {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// Why command ownership could not admit or be configured for a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CommandOwnerError {
    #[error("command concurrency must be between one and two")]
    InvalidConcurrency,
}

/// Why a new session did not enter runner supervision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CommandStartError {
    #[error("command owner reached its concurrency limit")]
    AtCapacity,
    #[error("command owner has stopped")]
    OwnerStopped,
    #[error("command output store could not be prepared")]
    OutputStore,
}

/// Removes the nonce directory one prepared store created if the start that
/// prepared it is refused: dropping the store's handle alone leaves the
/// directory behind. The guard removes exactly that directory — never any
/// sibling or ancestor — and only when the start was refused.
struct PreparedStoreGuard {
    store_root: PathBuf,
    nonce_dir: PathBuf,
    defused: bool,
}

impl PreparedStoreGuard {
    /// Arms the guard on a freshly prepared store's nonce directory.
    fn arm(store: &CommandOutputStore, store_root: PathBuf) -> Self {
        Self {
            store_root,
            nonce_dir: store.dir().to_path_buf(),
            defused: false,
        }
    }

    /// Hands the store to the started session: nothing is removed.
    fn defuse(mut self) {
        self.defused = true;
    }
}

impl Drop for PreparedStoreGuard {
    fn drop(&mut self) {
        if self.defused {
            return;
        }
        // The store's command-output dir sits directly under `store_root`,
        // and the nonce dir directly under it; refuse to touch a nonce path
        // with any other shape.
        let Some(parent) = self.nonce_dir.parent() else {
            return;
        };
        if parent.parent() != Some(self.store_root.as_path()) {
            return;
        }
        // A plain directory per `symlink_metadata` — a symlink to a
        // directory is not a directory. Anything missing or symlinked is
        // left alone.
        let plain_dir = |path: &Path| fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
        if !plain_dir(parent) || !plain_dir(&self.nonce_dir) {
            return;
        }
        let _ = fs::remove_dir_all(&self.nonce_dir);
    }
}
