use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::event_kinds::WireCodingEventKind;
use super::event_validation::validate_event;
use super::{AgentId, InvocationId, JobId};

pub use super::event_kinds::{ApprovalDisposition, CodingEventKind};

pub const CODING_EVENT_VERSION: u32 = 1;
pub const MAX_CODING_EVENT_BYTES: usize = 64 * 1024;

/// One strictly versioned event. Sequence allocation and durable ordering
/// belong to saya-store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodingEventV1 {
    version: u32,
    sequence: NonZeroU64,
    job_id: JobId,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<AgentId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invocation_id: Option<InvocationId>,
    event: CodingEventKind,
}

impl CodingEventV1 {
    pub fn new(
        sequence: NonZeroU64,
        job_id: JobId,
        agent_id: Option<AgentId>,
        invocation_id: Option<InvocationId>,
        event: CodingEventKind,
    ) -> Result<Self, CodingEventError> {
        let result_without_child = matches!(event, CodingEventKind::ResultReported { .. })
            && (agent_id.is_none() || invocation_id.is_none());
        if (invocation_id.is_some() && agent_id.is_none()) || result_without_child {
            return Err(CodingEventError::InvalidIdentity);
        }
        validate_event(
            sequence,
            &job_id,
            agent_id.as_ref(),
            invocation_id.as_ref(),
            &event,
        )?;
        let value = Self {
            version: CODING_EVENT_VERSION,
            sequence,
            job_id,
            agent_id,
            invocation_id,
            event,
        };
        value.check_size()?;
        Ok(value)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, CodingEventError> {
        if bytes.len() > MAX_CODING_EVENT_BYTES {
            return Err(CodingEventError::Oversize(bytes.len()));
        }
        let probe: VersionProbe =
            serde_json::from_slice(bytes).map_err(|_| CodingEventError::Malformed)?;
        if let Some(version) = probe.version.filter(|v| *v != CODING_EVENT_VERSION) {
            return Err(CodingEventError::UnsupportedVersion(version));
        }
        let raw: WireCodingEvent =
            serde_json::from_slice(bytes).map_err(|_| CodingEventError::Malformed)?;
        if raw.version != CODING_EVENT_VERSION {
            return Err(CodingEventError::UnsupportedVersion(raw.version));
        }
        Self::new(
            raw.sequence,
            raw.job_id,
            raw.agent_id,
            raw.invocation_id,
            raw.event.into_public()?,
        )
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, CodingEventError> {
        let bytes = serde_json::to_vec(self).map_err(|_| CodingEventError::Malformed)?;
        if bytes.len() > MAX_CODING_EVENT_BYTES {
            return Err(CodingEventError::Oversize(bytes.len()));
        }
        Ok(bytes)
    }

    fn check_size(&self) -> Result<(), CodingEventError> {
        let bytes = serde_json::to_vec(self).map_err(|_| CodingEventError::Malformed)?;
        if bytes.len() > MAX_CODING_EVENT_BYTES {
            return Err(CodingEventError::Oversize(bytes.len()));
        }
        Ok(())
    }

    pub fn sequence(&self) -> NonZeroU64 {
        self.sequence
    }

    pub fn job_id(&self) -> &JobId {
        &self.job_id
    }

    pub fn agent_id(&self) -> Option<&AgentId> {
        self.agent_id.as_ref()
    }

    pub fn invocation_id(&self) -> Option<&InvocationId> {
        self.invocation_id.as_ref()
    }

    pub fn event(&self) -> &CodingEventKind {
        &self.event
    }
}

#[derive(Deserialize)]
struct VersionProbe {
    version: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireCodingEvent {
    version: u32,
    sequence: NonZeroU64,
    job_id: JobId,
    #[serde(default)]
    agent_id: Option<AgentId>,
    #[serde(default)]
    invocation_id: Option<InvocationId>,
    event: WireCodingEventKind,
}

/// Why a coding event could not be constructed or decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum CodingEventError {
    #[error("coding event is {0} bytes, over the {MAX_CODING_EVENT_BYTES}-byte limit")]
    Oversize(usize),
    #[error("unsupported coding event version {0}")]
    UnsupportedVersion(u32),
    #[error("coding event is malformed")]
    Malformed,
    #[error("an invocation identity requires an agent identity")]
    InvalidIdentity,
    #[error("coding event source digest is not valid lowercase SHA-256")]
    InvalidDigest,
    #[error("coding event outer identity or sequence does not match its control record")]
    InvalidEventLink,
}
