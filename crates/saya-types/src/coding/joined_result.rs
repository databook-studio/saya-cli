//! Joined output is an untrusted report, never a source receipt or an authority grant.
use super::{AgentId, CodingFailureClass, CodingResultOutcome, InvocationId, JobId, JobState};
use serde::{Deserialize, Serialize};

pub const MAX_JOINED_ANSWER_BYTES: usize = 16 * 1024;
pub const MAX_JOINED_RESULT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JoinedResultV1 {
    version: u32,
    job_id: JobId,
    agent_id: AgentId,
    invocation_id: InvocationId,
    outcome: CodingResultOutcome,
    answer: String,
    answer_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure_class: Option<CodingFailureClass>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JoinedResultError {
    #[error("joined result exceeds its byte bound")]
    TooLarge,
    #[error("joined result is malformed or has an unsupported version")]
    Invalid,
}
impl JoinedResultV1 {
    pub fn new(
        job_id: JobId,
        agent_id: AgentId,
        invocation_id: InvocationId,
        outcome: CodingResultOutcome,
        answer: String,
        answer_truncated: bool,
    ) -> Result<Self, JoinedResultError> {
        if answer.len() > MAX_JOINED_ANSWER_BYTES {
            return Err(JoinedResultError::TooLarge);
        }
        let result = Self {
            version: 1,
            job_id,
            agent_id,
            invocation_id,
            outcome,
            answer,
            answer_truncated,
            failure_class: None,
        };
        result.to_json_bytes()?;
        Ok(result)
    }
    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }
    pub fn invocation_id(&self) -> &InvocationId {
        &self.invocation_id
    }
    pub fn outcome(&self) -> CodingResultOutcome {
        self.outcome
    }
    pub fn answer(&self) -> &str {
        &self.answer
    }
    pub fn answer_truncated(&self) -> bool {
        self.answer_truncated
    }
    /// Why a failed child stopped; absent for any other outcome and for results stored before it existed.
    pub fn failure_class(&self) -> Option<CodingFailureClass> {
        self.failure_class
    }
    pub fn with_failure_class(mut self, class: CodingFailureClass) -> Self {
        self.failure_class = Some(class);
        self
    }
    pub fn terminal_state(&self) -> JobState {
        match self.outcome {
            CodingResultOutcome::Completed => JobState::Completed,
            CodingResultOutcome::Failed => JobState::Failed,
            CodingResultOutcome::Cancelled => JobState::Cancelled,
            CodingResultOutcome::Uncertain => JobState::Uncertain,
        }
    }
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, JoinedResultError> {
        let bytes = serde_json::to_vec(self).map_err(|_| JoinedResultError::Invalid)?;
        if bytes.len() > MAX_JOINED_RESULT_BYTES {
            return Err(JoinedResultError::TooLarge);
        }
        Ok(bytes)
    }
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, JoinedResultError> {
        if bytes.len() > MAX_JOINED_RESULT_BYTES {
            return Err(JoinedResultError::TooLarge);
        }
        let wire: Wire = serde_json::from_slice(bytes).map_err(|_| JoinedResultError::Invalid)?;
        if wire.version != 1 {
            return Err(JoinedResultError::Invalid);
        }
        if wire.failure_class.is_some() && wire.outcome != CodingResultOutcome::Failed {
            return Err(JoinedResultError::Invalid);
        }
        let class = wire.failure_class;
        let result = Self::new(
            wire.job_id,
            wire.agent_id,
            wire.invocation_id,
            wire.outcome,
            wire.answer,
            wire.answer_truncated,
        )?;
        Ok(match class {
            Some(class) => result.with_failure_class(class),
            None => result,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    version: u32,
    job_id: JobId,
    agent_id: AgentId,
    invocation_id: InvocationId,
    outcome: CodingResultOutcome,
    answer: String,
    answer_truncated: bool,
    #[serde(default)]
    failure_class: Option<CodingFailureClass>,
}

#[cfg(test)]
#[path = "joined_result_tests.rs"]
mod tests;
