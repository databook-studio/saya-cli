use saya_agent::{AgentMailbox, AgentSteering, CancellationToken, SteeringError, agent_mailbox};

/// Trusted caller control for one invocation. No control handle enters child tools.
#[derive(Clone)]
pub struct ChildControl {
    pub(crate) cancellation: CancellationToken,
    steering: AgentSteering,
}
impl ChildControl {
    pub(crate) fn new() -> (Self, AgentMailbox) {
        let (steering, mailbox) = agent_mailbox();
        (
            Self {
                cancellation: CancellationToken::new(),
                steering,
            },
            mailbox,
        )
    }
    pub fn send(&self, text: &str) -> Result<(), SteeringError> {
        self.steering.send(text)
    }
    pub fn pause(&self) -> Result<u64, SteeringError> {
        self.steering.pause()
    }
    pub fn resume(&self) -> Result<(), SteeringError> {
        self.steering.resume()
    }
    pub fn is_paused(&self, ticket: u64) -> bool {
        self.steering.is_paused(ticket)
    }
    pub async fn wait_paused(&self, ticket: u64) -> bool {
        self.steering.wait_paused(ticket).await
    }
    /// Independent of steering capacity and pause state. The schedule still owns join.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

impl std::fmt::Debug for ChildControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChildControl { .. }")
    }
}
