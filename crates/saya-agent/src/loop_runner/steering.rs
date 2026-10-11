//! Bounded input delivered only between complete agent turns. No authority travels here.
use crate::{AgentError, CancellationToken, ChatMessage, ToolExecutor};
use tokio::sync::{mpsc, watch};

pub const MAX_STEERING_MESSAGES: usize = 16;
pub const MAX_STEERING_MESSAGE_BYTES: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SteeringError {
    #[error("steering mailbox is full")]
    Full,
    #[error("steering input exceeds its byte bound")]
    TooLarge,
    #[error("child invocation has ended")]
    Closed,
    #[error("pause request counter is exhausted")]
    Exhausted,
}
#[derive(Debug, Clone, Copy, Default)]
struct PauseRequest {
    epoch: u64,
    paused: bool,
}

#[derive(Debug, Clone)]
pub struct AgentSteering {
    sender: mpsc::Sender<String>,
    pause: watch::Sender<PauseRequest>,
    acknowledged: watch::Receiver<u64>,
}
/// Single receiver consumed by exactly one invocation; it cannot be cloned.
#[derive(Debug)]
pub struct AgentMailbox {
    receiver: mpsc::Receiver<String>,
    pause: watch::Receiver<PauseRequest>,
    acknowledged: watch::Sender<u64>,
}
pub fn agent_mailbox() -> (AgentSteering, AgentMailbox) {
    let (sender, receiver) = mpsc::channel(MAX_STEERING_MESSAGES);
    let (pause, pause_rx) = watch::channel(PauseRequest::default());
    let (acknowledged, ack_rx) = watch::channel(0);
    (
        AgentSteering {
            sender,
            pause,
            acknowledged: ack_rx,
        },
        AgentMailbox {
            receiver,
            pause: pause_rx,
            acknowledged,
        },
    )
}
impl AgentSteering {
    /// Enqueues untrusted user input. Success acknowledges queueing only.
    pub fn send(&self, text: &str) -> Result<(), SteeringError> {
        if text.len() > MAX_STEERING_MESSAGE_BYTES {
            return Err(SteeringError::TooLarge);
        }
        self.sender
            .try_send(text.to_owned())
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => SteeringError::Full,
                mpsc::error::TrySendError::Closed(_) => SteeringError::Closed,
            })
    }
    pub fn pause(&self) -> Result<u64, SteeringError> {
        self.change_pause(true)
    }
    pub fn resume(&self) -> Result<(), SteeringError> {
        self.change_pause(false).map(|_| ())
    }
    fn change_pause(&self, paused: bool) -> Result<u64, SteeringError> {
        if self.sender.is_closed() {
            return Err(SteeringError::Closed);
        }
        let mut result = Err(SteeringError::Exhausted);
        self.pause.send_if_modified(|request| {
            let Some(epoch) = request.epoch.checked_add(1) else {
                return false;
            };
            *request = PauseRequest { epoch, paused };
            result = Ok(epoch);
            true
        });
        result
    }
    pub fn is_paused(&self, ticket: u64) -> bool {
        let requested = *self.pause.borrow();
        !self.sender.is_closed()
            && requested.paused
            && requested.epoch == ticket
            && *self.acknowledged.borrow() == ticket
    }
    /// False means this pause was superseded or the invocation ended.
    pub async fn wait_paused(&self, ticket: u64) -> bool {
        let mut acknowledged = self.acknowledged.clone();
        let mut requested = self.pause.subscribe();
        loop {
            let state = *requested.borrow_and_update();
            if !state.paused || state.epoch != ticket || self.sender.is_closed() {
                return false;
            }
            if *acknowledged.borrow_and_update() == ticket {
                return true;
            }
            tokio::select! {
                result = acknowledged.changed() => if result.is_err() { return false; },
                result = requested.changed() => if result.is_err() { return false; },
                _ = self.sender.closed() => return false,
            }
        }
    }
}
impl AgentMailbox {
    pub(super) async fn boundary(
        &mut self,
        token: &CancellationToken,
        messages: &mut Vec<ChatMessage>,
        tools: &dyn ToolExecutor,
    ) -> Result<(), AgentError> {
        loop {
            super::check_cancelled(token)?;
            let requested = *self.pause.borrow_and_update();
            if !requested.paused {
                break;
            }
            // A completed start/read tool can still own a background command.
            // Pause acknowledges only after that executor settles retained work.
            tools
                .settle_invocation()
                .await
                .map_err(crate::EffectOwnerFailure::new)?;
            super::check_cancelled(token)?;
            let current = *self.pause.borrow();
            if current.epoch != requested.epoch {
                continue;
            }
            self.acknowledged.send_replace(requested.epoch);
            tokio::select! {
                biased;
                _ = token.cancelled() => return Err(AgentError::Cancelled),
                changed = self.pause.changed() => if changed.is_err() { break; },
            }
        }
        self.acknowledged.send_replace(0);
        // A bounded snapshot prevents a producer from keeping this loop busy forever.
        for _ in 0..MAX_STEERING_MESSAGES {
            let Ok(text) = self.receiver.try_recv() else {
                break;
            };
            messages.push(ChatMessage::text("user", text));
        }
        Ok(())
    }
}
