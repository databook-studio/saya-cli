use super::{ProbeOutcome, add_tool_loop_failure, tool_loop_blocked, tool_loop_failure_field};
use serde_json::json;

fn probe(completed: bool, truncated: bool) -> Option<ProbeOutcome> {
    Some(ProbeOutcome {
        completed,
        truncated_by_output_limit: truncated,
    })
}

#[test]
fn a_truncated_probe_adds_the_output_limit_failure_field_naming_the_configured_number() {
    let field = tool_loop_failure_field(probe(false, true), 4096)
        .expect("a probe the output limit cut off must carry the failure field");
    assert_eq!(field["reason"], "output_limit");
    assert_eq!(field["configured_max_output_tokens"], 4096);
    assert_eq!(
        field["action"],
        "raise ai.max_output_tokens in trusted configuration"
    );
    let field = tool_loop_failure_field(probe(false, true), 8192)
        .expect("the failure field is built for the configured limit, whatever it is");
    assert_eq!(field["configured_max_output_tokens"], 8192);
}

#[test]
fn a_healthy_or_differently_failed_probe_builds_no_failure_field() {
    assert!(tool_loop_failure_field(probe(true, false), 4096).is_none());
    assert!(tool_loop_failure_field(probe(false, false), 4096).is_none());
    assert!(tool_loop_failure_field(None, 4096).is_none());
}

#[test]
fn the_doctor_json_carries_the_failure_field_only_for_a_truncated_probe() {
    let truncated = add_tool_loop_failure(
        json!({"ready": false}),
        tool_loop_failure_field(probe(false, true), 4096),
    );
    assert_eq!(
        truncated["native_provider_tool_loop_failure"]["reason"],
        "output_limit"
    );
    assert_eq!(
        truncated["native_provider_tool_loop_failure"]["configured_max_output_tokens"],
        4096
    );
    let healthy = add_tool_loop_failure(
        json!({"ready": true}),
        tool_loop_failure_field(probe(true, false), 4096),
    );
    assert!(
        healthy.get("native_provider_tool_loop_failure").is_none(),
        "a healthy probe must not carry the field: {healthy}"
    );
    let other = add_tool_loop_failure(
        json!({"ready": false}),
        tool_loop_failure_field(probe(false, false), 4096),
    );
    assert!(
        other.get("native_provider_tool_loop_failure").is_none(),
        "a probe that failed for another reason must not carry the field: {other}"
    );
}

#[test]
fn a_truncated_probe_blocks_with_the_output_limit_action_instead_of_the_generic_text() {
    let entry = tool_loop_blocked(probe(false, true), 4096)
        .expect("a probe the output limit cut off must block readiness");
    assert!(entry.contains("output-token limit"), "{entry}");
    assert!(entry.contains("ai.max_output_tokens = 4096"), "{entry}");
    assert!(
        entry.contains("raise ai.max_output_tokens in trusted configuration"),
        "{entry}"
    );
    assert!(entry.contains("coding doctor --probe"), "{entry}");
}

#[test]
fn a_healthy_probe_blocks_nothing_and_every_other_case_keeps_the_generic_text() {
    assert_eq!(tool_loop_blocked(probe(true, false), 4096), None);
    let generic = "Run coding doctor --probe and verify the configured provider completes the native tool loop";
    assert_eq!(tool_loop_blocked(None, 4096).as_deref(), Some(generic));
    assert_eq!(
        tool_loop_blocked(probe(false, false), 4096).as_deref(),
        Some(generic)
    );
}
