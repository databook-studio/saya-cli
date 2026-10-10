use std::{num::NonZeroU64, sync::Arc};

use saya_agent::CancellationToken;
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::runner::RunProgram;

use super::CommandSession;

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
        if self.cancellation.is_cancelled() {
            return Err(CommandStartError::OwnerStopped);
        }
        let permit = Arc::clone(&self.active)
            .try_acquire_owned()
            .map_err(|_| CommandStartError::AtCapacity)?;
        if self.cancellation.is_cancelled() {
            return Err(CommandStartError::OwnerStopped);
        }
        Ok(CommandSession::spawn(
            runner,
            program,
            argv,
            timeout_seconds,
            self.cancellation.clone(),
            permit,
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
}
