use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{AgentId, CodingResultOutcome, InvocationId, JobId, JobState, Sha256Digest};

pub const CODING_EVENT_VERSION: u32 = 1;
pub const MAX_CODING_EVENT_BYTES: usize = 64 * 1024;

/// A typed coding event payload. State transition policy remains with storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum CodingEventKind {
    StateChanged {
        state: JobState,
    },
    ResultReported {
        source_sha256: Sha256Digest,
        outcome: CodingResultOutcome,
    },
}

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
        Ok(Self {
            version: CODING_EVENT_VERSION,
            sequence,
            job_id,
            agent_id,
            invocation_id,
            event,
        })
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
        let event = match raw.event {
            WireCodingEventKind::StateChanged { state } => CodingEventKind::StateChanged { state },
            WireCodingEventKind::ResultReported {
                source_sha256,
                outcome,
            } => CodingEventKind::ResultReported {
                source_sha256: Sha256Digest::parse(&source_sha256)
                    .map_err(|_| CodingEventError::InvalidDigest)?,
                outcome,
            },
        };
        Self::new(
            raw.sequence,
            raw.job_id,
            raw.agent_id,
            raw.invocation_id,
            event,
        )
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, CodingEventError> {
        serde_json::to_vec(self).map_err(|_| CodingEventError::Malformed)
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

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireCodingEventKind {
    StateChanged {
        state: JobState,
    },
    ResultReported {
        source_sha256: String,
        outcome: CodingResultOutcome,
    },
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
}
