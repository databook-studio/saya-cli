use saya_agent::CancellationToken;
use thiserror::Error;

pub use crate::runner::stdin_channel::{
    MAX_COMMAND_INPUT_BYTES, MAX_COMMAND_INPUT_CHUNK_BYTES, MAX_COMMAND_INPUT_QUEUE_CHUNKS,
};
use crate::runner::stdin_channel::{
    StdinInputPipe, StdinInputSender, StdinSendError, input_channel,
};

/// Receipt that bytes entered the bounded session queue; it does not claim
/// that the kernel pipe accepted them or that the child consumed them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdinQueueReceipt {
    bytes: usize,
}

impl StdinQueueReceipt {
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// Why command-session stdin could not accept a chunk or close its stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CommandInputError {
    #[error("this command session was not started with stdin enabled")]
    Unavailable,
    #[error("command stdin is closed or its process has finished")]
    Closed,
    #[error("command stdin chunk exceeds the {max_bytes}-byte limit")]
    ChunkTooLarge { max_bytes: usize },
    #[error("command stdin queue is full (at most {max_chunks} chunks)")]
    QueueFull { max_chunks: usize },
    #[error("command stdin exceeds the {max_bytes}-byte lifetime limit")]
    LifetimeLimitExceeded { max_bytes: usize },
}

pub(super) struct CommandInput {
    sender: StdinInputSender,
}

impl CommandInput {
    pub(super) fn channel(owner_cancellation: CancellationToken) -> (Self, StdinInputPipe) {
        let (sender, receiver) = input_channel(owner_cancellation);
        (Self { sender }, receiver)
    }

    pub(super) fn send(&self, bytes: &[u8]) -> Result<StdinQueueReceipt, CommandInputError> {
        self.sender
            .try_send(bytes)
            .map(|bytes| StdinQueueReceipt { bytes })
            .map_err(map_send_error)
    }

    pub(super) fn close(&self) -> Result<(), CommandInputError> {
        self.sender.close().map_err(|_| CommandInputError::Closed)
    }

    pub(super) fn close_if_open(&self) {
        let _ = self.close();
    }
}

fn map_send_error(error: StdinSendError) -> CommandInputError {
    match error {
        StdinSendError::Closed => CommandInputError::Closed,
        StdinSendError::ChunkTooLarge => CommandInputError::ChunkTooLarge {
            max_bytes: MAX_COMMAND_INPUT_CHUNK_BYTES,
        },
        StdinSendError::QueueFull => CommandInputError::QueueFull {
            max_chunks: MAX_COMMAND_INPUT_QUEUE_CHUNKS,
        },
        StdinSendError::LifetimeLimitExceeded => CommandInputError::LifetimeLimitExceeded {
            max_bytes: MAX_COMMAND_INPUT_BYTES,
        },
    }
}
