//! Bounded source transport timing; never permission or proof of remote execution.
use super::thinking::ThinkingSpanData;
use super::{AgentId, InvocationId, JobId, Sha256Digest};
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
    /// Provider-reported breakdowns of the totals above, never additions to
    /// them: cached input is a subset of input, reasoning is counted inside
    /// output. Absent means the provider did not report the breakdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_reasoning_tokens: Option<u64>,
    /// Provider-reported cost in micro-USD, when the provider reports one
    /// (OpenRouter's `usage.cost`). Absent means unreported, never zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_cost_micros: Option<u64>,
    pub usage_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_correlation_id: Option<NonZeroU64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_correlation_id: Option<NonZeroU64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_correlation_sha256: Option<Sha256Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_ordinal: Option<NonZeroU64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_revision: Option<NonZeroU64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_size_bytes: Option<NonZeroU64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_key_digest: Option<u64>,
    /// The thinking control this request asked for, when it asked for one.
    /// Absent means the request sent no thinking field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingSpanData>,
    /// Whether the request asked the provider to stream. Absent for a
    /// historical span that did not record it; a present value is the wire's
    /// own `stream` flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamed: Option<bool>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TransportSpanV1(TransportSpanData);
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid or oversized transport observation")]
pub struct TransportSpanError;
impl TransportSpanV1 {
    pub fn new(data: TransportSpanData) -> Result<Self, TransportSpanError> {
        let correlation = [
            data.conversation_correlation_id.is_some(),
            data.invocation_correlation_id.is_some(),
            data.request_ordinal.is_some(),
            data.projection_revision.is_some(),
            data.projection_size_bytes.is_some(),
        ];
        let correlated = correlation.iter().any(|present| *present);
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
            || ((data.reported_cached_input_tokens.is_some()
                || data.reported_reasoning_tokens.is_some())
                && data.reported_input_tokens.is_none())
            || data
                .reported_cached_input_tokens
                .zip(data.reported_input_tokens)
                .is_some_and(|(cached, input)| cached > input)
            || data
                .reported_reasoning_tokens
                .zip(data.reported_output_tokens)
                .is_some_and(|(reasoning, output)| reasoning > output)
            || correlation.iter().any(|present| *present)
                != correlation.iter().all(|present| *present)
            || (!correlated
                && (data.actor_correlation_sha256.is_some() || data.cache_key_digest.is_some()))
            || data.thinking.is_some_and(|thinking| !thinking.is_valid())
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

#[cfg(test)]
mod tests {
    use super::TransportSpanV1;
    use serde_json::{Value, json};

    fn legacy_span() -> Value {
        json!({
            "version": 1,
            "job_id": "job-a",
            "agent_id": null,
            "invocation_id": null,
            "runtime_epoch": null,
            "reservation_sequence": 2,
            "parent_reservation_sequence": 1,
            "ordinal": 0,
            "call_kind": "answer",
            "process_id": 42,
            "clock_domain": "p42-t1",
            "dispatch_started_ns": 1,
            "finished_ns": 2,
            "outcome": "completed",
            "reported_input_tokens": null,
            "reported_output_tokens": null,
            "usage_complete": false
        })
    }

    #[test]
    fn version_one_correlation_is_optional_bounded_and_round_trips() {
        let legacy = TransportSpanV1::from_json_bytes(&serde_json::to_vec(&legacy_span()).unwrap())
            .expect("historical V1 span remains readable");
        assert!(legacy.data().conversation_correlation_id.is_none());
        assert!(legacy.data().invocation_correlation_id.is_none());
        assert!(legacy.data().actor_correlation_sha256.is_none());
        assert!(legacy.data().request_ordinal.is_none());
        assert!(legacy.data().projection_revision.is_none());
        assert!(legacy.data().projection_size_bytes.is_none());
        assert!(legacy.data().cache_key_digest.is_none());
        let legacy_wire: Value = serde_json::from_slice(&legacy.to_json_bytes().unwrap()).unwrap();
        assert!(legacy_wire.get("conversation_correlation_id").is_none());

        let mut correlated = legacy_span();
        correlated["conversation_correlation_id"] = json!(3);
        correlated["invocation_correlation_id"] = json!(4);
        correlated["actor_correlation_sha256"] = json!("a".repeat(64));
        correlated["request_ordinal"] = json!(1);
        correlated["projection_revision"] = json!(8);
        correlated["projection_size_bytes"] = json!(1024);
        correlated["cache_key_digest"] = json!(74565);
        let span = TransportSpanV1::from_json_bytes(&serde_json::to_vec(&correlated).unwrap())
            .expect("complete bounded correlation should deserialize");
        let wire: Value = serde_json::from_slice(&span.to_json_bytes().unwrap()).unwrap();
        for field in [
            "conversation_correlation_id",
            "invocation_correlation_id",
            "actor_correlation_sha256",
            "request_ordinal",
            "projection_revision",
            "projection_size_bytes",
            "cache_key_digest",
        ] {
            assert_eq!(wire[field], correlated[field], "field {field} is durable");
        }

        let mut partial = legacy_span();
        partial["conversation_correlation_id"] = json!(3);
        assert!(TransportSpanV1::from_json_bytes(&serde_json::to_vec(&partial).unwrap()).is_err());

        let mut zero = correlated;
        zero["request_ordinal"] = json!(0);
        assert!(TransportSpanV1::from_json_bytes(&serde_json::to_vec(&zero).unwrap()).is_err());
    }

    #[test]
    fn cache_and_reasoning_usage_are_optional_bounded_and_round_trip() {
        let mut reported = legacy_span();
        reported["reported_input_tokens"] = json!(100);
        reported["reported_output_tokens"] = json!(40);
        reported["usage_complete"] = json!(true);
        reported["reported_cached_input_tokens"] = json!(90);
        reported["reported_reasoning_tokens"] = json!(10);
        let span =
            TransportSpanV1::from_json_bytes(&serde_json::to_vec(&reported).unwrap()).unwrap();
        let wire: Value = serde_json::from_slice(&span.to_json_bytes().unwrap()).unwrap();
        assert_eq!(wire["reported_cached_input_tokens"], 90);
        assert_eq!(wire["reported_reasoning_tokens"], 10);

        // A historical span without the breakdowns stays readable and does
        // not gain fields it never reported.
        let legacy =
            TransportSpanV1::from_json_bytes(&serde_json::to_vec(&legacy_span()).unwrap()).unwrap();
        let wire: Value = serde_json::from_slice(&legacy.to_json_bytes().unwrap()).unwrap();
        assert!(wire.get("reported_cached_input_tokens").is_none());
        assert!(wire.get("reported_reasoning_tokens").is_none());
    }

    #[test]
    fn usage_breakdowns_cannot_exceed_their_totals() {
        let mut span = legacy_span();
        span["reported_input_tokens"] = json!(100);
        span["reported_output_tokens"] = json!(40);
        span["usage_complete"] = json!(true);
        span["reported_cached_input_tokens"] = json!(101);
        assert!(TransportSpanV1::from_json_bytes(&serde_json::to_vec(&span).unwrap()).is_err());

        span["reported_cached_input_tokens"] = json!(100);
        span["reported_reasoning_tokens"] = json!(41);
        assert!(TransportSpanV1::from_json_bytes(&serde_json::to_vec(&span).unwrap()).is_err());

        // A breakdown without the totals it breaks down is not a measurement.
        span["reported_reasoning_tokens"] = json!(0);
        span["reported_input_tokens"] = Value::Null;
        span["reported_output_tokens"] = Value::Null;
        span["usage_complete"] = json!(false);
        assert!(TransportSpanV1::from_json_bytes(&serde_json::to_vec(&span).unwrap()).is_err());
    }
}
