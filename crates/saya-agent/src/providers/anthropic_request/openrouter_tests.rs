// The OpenRouter Anthropic route's thinking shape, kept as it was at base.

use super::*;
use crate::ChatMessage;

/// OpenRouter's Anthropic route sends the same adaptive thinking shape the
/// native route sent at base: every policy effort — high, the new medium
/// escalation tier, and the routine low — arrives as `thinking.adaptive` plus
/// `output_config.effort`, with no temperature.
#[test]
fn openrouter_policy_efforts_use_adaptive_thinking() {
    for (effort, spelling) in [
        (ReasoningEffort::High, "high"),
        (ReasoningEffort::Medium, "medium"),
        (ReasoningEffort::Low, "low"),
    ] {
        let request = ChatRequest {
            model: "anthropic/claude-sonnet-4.5".into(),
            messages: vec![ChatMessage::text("user", "hello")],
            tools: Vec::new(),
            reasoning_effort: effort,
            ..Default::default()
        };
        let body = build_body(request, 32_000, Some(0.1), EffortSource::Policy, true);
        assert_eq!(
            body["thinking"],
            json!({"type": "adaptive"}),
            "{spelling} effort must use adaptive thinking: {body}"
        );
        assert_eq!(
            body["output_config"],
            json!({"effort": spelling}),
            "{spelling} effort must ride output_config: {body}"
        );
        assert!(
            body.get("temperature").is_none(),
            "adaptive thinking rejects temperature: {body}"
        );
    }
}

/// An operator-stated effort on the OpenRouter route keeps the base shape: the
/// fixed thinking budget, the temperature, and no adaptive beta. Only the
/// loop's policy hints use the route's adaptive control.
#[test]
fn openrouter_stated_efforts_keep_their_thinking_budget() {
    for (effort, budget) in [
        (ReasoningEffort::Low, 2048),
        (ReasoningEffort::Medium, 4096),
    ] {
        let request = ChatRequest {
            model: "anthropic/claude-sonnet-4.5".into(),
            messages: vec![ChatMessage::text("user", "hello")],
            tools: Vec::new(),
            reasoning_effort: effort,
            ..Default::default()
        };
        let body = build_body(request, 32_000, Some(0.1), EffortSource::Stated, true);
        assert_eq!(
            body["thinking"],
            json!({"type": "enabled", "budget_tokens": budget}),
            "{effort:?} is stated, not a policy hint"
        );
        assert!(
            body.get("output_config").is_none(),
            "only adaptive requests carry output_config: {body}"
        );
        assert_eq!(body["temperature"], json!(0.1f32));
    }
}
