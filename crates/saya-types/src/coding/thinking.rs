//! The thinking control a request asked the provider for, as a bounded span
//! observation. Never a measurement of what the model actually thought.
use serde::{Deserialize, Serialize};

/// The largest thinking budget a span may report. The value is the request's
/// own token budget, not a provider ceiling; the bound keeps a corrupt or
/// hostile observation from claiming an implausible one.
pub const MAX_THINKING_BUDGET_TOKENS: u32 = 1_000_000;

/// How a request asked the provider to reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    /// Adaptive thinking: the model decides how much to think, bounded by an
    /// effort setting.
    Adaptive,
    /// A fixed `budget_tokens` allowance.
    Enabled,
}

/// The effort setting sent with adaptive thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingEffort {
    Low,
    Medium,
    High,
}

/// What one request asked of the provider's thinking control. Optional as a
/// whole: a request that sent no thinking field reports none, which is not the
/// same as reporting a zero budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThinkingSpanData {
    pub mode: ThinkingMode,
    /// The fixed allowance; present only for [`ThinkingMode::Enabled`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
    /// The effort setting; present only for [`ThinkingMode::Adaptive`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ThinkingEffort>,
}

impl ThinkingSpanData {
    /// A mode and its matching control, never both or neither. A budget of
    /// zero is not a measurement of "no thinking"; it is invalid.
    pub(crate) fn is_valid(self) -> bool {
        match self.mode {
            ThinkingMode::Adaptive => self.budget_tokens.is_none() && self.effort.is_some(),
            ThinkingMode::Enabled => {
                self.budget_tokens
                    .is_some_and(|budget| (1..=MAX_THINKING_BUDGET_TOKENS).contains(&budget))
                    && self.effort.is_none()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_THINKING_BUDGET_TOKENS, ThinkingEffort, ThinkingMode, ThinkingSpanData};
    use crate::coding::transport::TransportSpanV1;
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
    fn thinking_control_and_stream_flag_are_optional_bounded_and_round_trip() {
        let mut adaptive = legacy_span();
        adaptive["thinking"] = json!({"mode": "adaptive", "effort": "low"});
        adaptive["streamed"] = json!(true);
        let span = TransportSpanV1::from_json_bytes(&serde_json::to_vec(&adaptive).unwrap())
            .expect("adaptive thinking is a valid bounded observation");
        let wire: Value = serde_json::from_slice(&span.to_json_bytes().unwrap()).unwrap();
        assert_eq!(
            wire["thinking"],
            json!({"mode": "adaptive", "effort": "low"})
        );
        assert_eq!(wire["streamed"], true);

        let mut enabled = legacy_span();
        enabled["thinking"] = json!({"mode": "enabled", "budget_tokens": 2048});
        let span = TransportSpanV1::from_json_bytes(&serde_json::to_vec(&enabled).unwrap())
            .expect("a fixed budget is a valid bounded observation");
        let wire: Value = serde_json::from_slice(&span.to_json_bytes().unwrap()).unwrap();
        assert_eq!(
            wire["thinking"],
            json!({"mode": "enabled", "budget_tokens": 2048})
        );

        // A historical span reports neither and gains no field it never had.
        let legacy = TransportSpanV1::from_json_bytes(&serde_json::to_vec(&legacy_span()).unwrap())
            .expect("historical V1 span remains readable");
        let wire: Value = serde_json::from_slice(&legacy.to_json_bytes().unwrap()).unwrap();
        assert!(wire.get("thinking").is_none());
        assert!(wire.get("streamed").is_none());
    }

    #[test]
    fn a_mode_without_its_matching_control_is_not_an_observation() {
        for invalid in [
            json!({"mode": "adaptive"}),
            json!({"mode": "adaptive", "budget_tokens": 2048}),
            json!({"mode": "enabled"}),
            json!({"mode": "enabled", "budget_tokens": 0}),
            json!({"mode": "enabled", "budget_tokens": MAX_THINKING_BUDGET_TOKENS + 1}),
            json!({"mode": "enabled", "budget_tokens": 2048, "effort": "low"}),
        ] {
            let mut span = legacy_span();
            span["thinking"] = invalid.clone();
            assert!(
                TransportSpanV1::from_json_bytes(&serde_json::to_vec(&span).unwrap()).is_err(),
                "invalid thinking observation must be refused: {invalid}"
            );
        }
    }

    #[test]
    fn the_matching_controls_are_valid() {
        assert!(
            ThinkingSpanData {
                mode: ThinkingMode::Adaptive,
                budget_tokens: None,
                effort: Some(ThinkingEffort::High),
            }
            .is_valid()
        );
        assert!(
            ThinkingSpanData {
                mode: ThinkingMode::Enabled,
                budget_tokens: Some(1024),
                effort: None,
            }
            .is_valid()
        );
    }
}
