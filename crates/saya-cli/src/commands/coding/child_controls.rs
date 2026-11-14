use super::{CodingError, NativeCodingJob};
use saya_agent::SteeringError;
use saya_types::AgentId;
use std::sync::Arc;

impl NativeCodingJob {
    pub async fn control_command(
        self: &Arc<Self>,
        command: crate::cli::CodingCommand,
    ) -> Result<serde_json::Value, CodingError> {
        use crate::cli::CodingCommand;
        if self.cancellation.is_cancelled() {
            return Err(CodingError::NotOwned);
        }
        if let CodingCommand::FollowUp {
            id,
            agent,
            expected_completion_sequence,
            instruction,
            ..
        } = &command
        {
            if id != self.job_id().as_str() {
                return Err(CodingError::NotOwned);
            }
            let request = super::ChildFollowUpRequest {
                agent_id: AgentId::parse(agent).map_err(CodingError::owner)?,
                expected_completion_sequence: std::num::NonZeroU64::new(
                    *expected_completion_sequence,
                )
                .ok_or(CodingError::InvalidRequest)?,
                instruction: instruction.clone(),
            };
            return serde_json::to_value(self.follow_up(request).await?)
                .map_err(CodingError::owner);
        }
        let (id, agent) = match &command {
            CodingCommand::Message { id, agent, .. }
            | CodingCommand::Pause { id, agent }
            | CodingCommand::Continue { id, agent }
            | CodingCommand::CancelChild { id, agent } => (id, agent),
            _ => return Err(CodingError::InvalidRequest),
        };
        if id != self.job_id().as_str() {
            return Err(CodingError::NotOwned);
        }
        if agent == "coordinator" {
            if self.root_mailbox.lock().await.is_some() {
                return Err(CodingError::NotOwned);
            }
            return match command {
                CodingCommand::Message { text, .. } => {
                    self.root_steering.send(&text).map_err(control_error)?;
                    Ok(serde_json::json!({"queued":true,"target":"coordinator"}))
                }
                CodingCommand::Pause { .. } => {
                    let ticket = self.root_steering.pause().map_err(control_error)?;
                    Ok(
                        serde_json::json!({"pause_requested":ticket,"acknowledged":false,"target":"coordinator"}),
                    )
                }
                CodingCommand::Continue { .. } => {
                    self.root_steering.resume().map_err(control_error)?;
                    Ok(serde_json::json!({"continue_requested":true,"target":"coordinator"}))
                }
                _ => Err(CodingError::InvalidRequest),
            };
        }
        let agent = saya_types::AgentId::parse(agent).map_err(CodingError::owner)?;
        match command {
            CodingCommand::Message { text, .. } => {
                self.message(&agent, &text).await?;
                Ok(serde_json::json!({"queued":true}))
            }
            CodingCommand::Pause { .. } => {
                let ticket = self.pause(&agent).await?;
                Ok(serde_json::json!({"pause_requested":ticket,"acknowledged":false}))
            }
            CodingCommand::Continue { .. } => {
                self.resume_child(&agent).await?;
                Ok(serde_json::json!({"continue_requested":true}))
            }
            CodingCommand::CancelChild { .. } => {
                self.cancel_child(&agent).await?;
                Ok(serde_json::json!({"cancel_requested":true}))
            }
            _ => Err(CodingError::InvalidRequest),
        }
    }

    pub async fn wait_pause(
        &self,
        id: &str,
        agent: &str,
        ticket: u64,
    ) -> Result<bool, CodingError> {
        if id != self.job_id().as_str() || self.cancellation.is_cancelled() {
            return Err(CodingError::NotOwned);
        }
        if agent == "coordinator" {
            if self.root_mailbox.lock().await.is_some() {
                return Err(CodingError::NotOwned);
            }
            return Ok(self.root_steering.wait_paused(ticket).await);
        }
        let agent = AgentId::parse(agent).map_err(CodingError::owner)?;
        Ok(self.control(&agent).await?.wait_paused(ticket).await)
    }
}

pub(super) fn control_error(error: SteeringError) -> CodingError {
    if matches!(error, SteeringError::Closed) {
        CodingError::NotOwned
    } else {
        CodingError::owner(error)
    }
}
