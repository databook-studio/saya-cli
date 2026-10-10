use saya_agent::CancellationToken;
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
};

pub const MAX_COMMAND_INPUT_CHUNK_BYTES: usize = 8 * 1024;
pub const MAX_COMMAND_INPUT_QUEUE_CHUNKS: usize = 16;
pub const MAX_COMMAND_INPUT_BYTES: usize = 1024 * 1024;

pub(crate) struct StdinInputSender {
    queue: Arc<InputQueue>,
    owner_cancellation: CancellationToken,
}

pub(crate) struct StdinInputPipe {
    queue: Arc<InputQueue>,
}

impl Drop for StdinInputPipe {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) enum StdinSendError {
    Closed,
    ChunkTooLarge,
    QueueFull,
    LifetimeLimitExceeded,
}

struct InputQueue {
    state: Mutex<QueueState>,
    changed: Condvar,
}

struct QueueState {
    chunks: VecDeque<Vec<u8>>,
    accepted_bytes: usize,
    closed: bool,
}

pub(crate) fn input_channel(
    owner_cancellation: CancellationToken,
) -> (StdinInputSender, StdinInputPipe) {
    let queue = Arc::new(InputQueue {
        state: Mutex::new(QueueState {
            chunks: VecDeque::new(),
            accepted_bytes: 0,
            closed: false,
        }),
        changed: Condvar::new(),
    });
    (
        StdinInputSender {
            queue: Arc::clone(&queue),
            owner_cancellation,
        },
        StdinInputPipe { queue },
    )
}

impl StdinInputSender {
    pub(crate) fn try_send(&self, bytes: &[u8]) -> Result<usize, StdinSendError> {
        let mut state = self.lock();
        if self.owner_cancellation.is_cancelled() {
            state.closed = true;
            self.queue.changed.notify_all();
        }
        if state.closed {
            return Err(StdinSendError::Closed);
        }
        if bytes.len() > MAX_COMMAND_INPUT_CHUNK_BYTES {
            return Err(StdinSendError::ChunkTooLarge);
        }
        if bytes.is_empty() {
            return Ok(0);
        }
        let next = state
            .accepted_bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= MAX_COMMAND_INPUT_BYTES)
            .ok_or(StdinSendError::LifetimeLimitExceeded)?;
        if state.chunks.len() >= MAX_COMMAND_INPUT_QUEUE_CHUNKS {
            return Err(StdinSendError::QueueFull);
        }
        state.chunks.push_back(bytes.to_vec());
        state.accepted_bytes = next;
        self.queue.changed.notify_one();
        Ok(bytes.len())
    }

    pub(crate) fn close(&self) -> Result<(), StdinSendError> {
        let mut state = self.lock();
        if state.closed {
            return Err(StdinSendError::Closed);
        }
        state.closed = true;
        self.queue.changed.notify_all();
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.queue
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl StdinInputPipe {
    pub(crate) fn clone_for_writer(&self) -> Self {
        Self {
            queue: Arc::clone(&self.queue),
        }
    }

    pub(crate) fn accepted_bytes(&self) -> usize {
        self.lock().accepted_bytes
    }

    pub(crate) fn close_and_accepted_bytes(&self) -> usize {
        let mut state = self.lock();
        state.closed = true;
        self.queue.changed.notify_all();
        state.accepted_bytes
    }

    pub(crate) fn next_chunk(
        &self,
        stop: &std::sync::atomic::AtomicBool,
        poll: std::time::Duration,
    ) -> Option<Vec<u8>> {
        let mut state = self.lock();
        loop {
            if stop.load(std::sync::atomic::Ordering::Acquire) {
                return None;
            }
            if let Some(chunk) = state.chunks.pop_front() {
                return Some(chunk);
            }
            if state.closed {
                return None;
            }
            state = self
                .queue
                .changed
                .wait_timeout(state, poll)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    pub(crate) fn stop(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.chunks.clear();
        self.queue.changed.notify_all();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, QueueState> {
        self.queue
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
