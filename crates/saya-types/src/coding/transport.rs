//! Bounded source transport timing; never permission or proof of remote execution.
use super::{AgentId, InvocationId, JobId};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

pub const MAX_TRANSPORT_SPAN_BYTES: usize = 4_096;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportOutcome {
    NotStarted,
    Completed,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportCallKind {
    Answer,
    Continuation,
    Salvage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportSpanData {
    pub version: u32,
    pub job_id: JobId,
    pub agent_id: Option<AgentId>,
    pub invocation_id: Option<InvocationId>,
    pub runtime_epoch: Option<NonZeroU64>,
    pub reservation_sequence: NonZeroU64,
    pub parent_reservation_sequence: NonZeroU64,
    pub ordinal: u8,
    pub call_kind: TransportCallKind,
    pub process_id: u32,
    pub clock_domain: String,
    pub dispatch_started_ns: Option<u64>,
    pub finished_ns: u64,
    pub outcome: TransportOutcome,
    pub reported_input_tokens: Option<u64>,
    pub reported_output_tokens: Option<u64>,
    pub usage_complete: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TransportSpanV1(TransportSpanData);
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid or oversized transport observation")]
pub struct TransportSpanError;
impl TransportSpanV1 {
    pub fn new(data: TransportSpanData) -> Result<Self, TransportSpanError> {
        if data.version != 1
            || data.agent_id.is_some() != data.invocation_id.is_some()
            || data.parent_reservation_sequence >= data.reservation_sequence
            || data.ordinal >= 16
            || data.process_id == 0
            || data.clock_domain.is_empty()
            || data.clock_domain.len() > 96
            || !data
                .clock_domain
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || data
                .dispatch_started_ns
                .is_some_and(|start| start > data.finished_ns)
            || (data.outcome == TransportOutcome::NotStarted) != data.dispatch_started_ns.is_none()
            || data.reported_input_tokens.is_some() != data.reported_output_tokens.is_some()
            || (data.usage_complete && data.reported_input_tokens.is_none())
        {
            return Err(TransportSpanError);
        }
        let span = Self(data);
        span.to_json_bytes()?;
        Ok(span)
    }
    pub fn data(&self) -> &TransportSpanData {
        &self.0
    }
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, TransportSpanError> {
        let bytes = serde_json::to_vec(self).map_err(|_| TransportSpanError)?;
        if bytes.len() > MAX_TRANSPORT_SPAN_BYTES {
            return Err(TransportSpanError);
        }
        Ok(bytes)
    }
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, TransportSpanError> {
        if bytes.len() > MAX_TRANSPORT_SPAN_BYTES {
            return Err(TransportSpanError);
        }
        serde_json::from_slice(bytes).map_err(|_| TransportSpanError)
    }
}
impl<'de> Deserialize<'de> for TransportSpanV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(TransportSpanData::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
