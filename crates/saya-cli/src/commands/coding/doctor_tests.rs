use super::{
    DoctorFacts, ProbeOutcome, add_tool_loop_failure, report, tool_loop_blocked,
    tool_loop_failure_field,
};
use saya_config::ResolvedCoding;
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn probe(completed: bool, truncated: bool) -> Option<ProbeOutcome> {
    Some(ProbeOutcome {
        completed,
        truncated_by_output_limit: truncated,
    })
}

fn facts(coding: ResolvedCoding) -> DoctorFacts {
    DoctorFacts {
        provider_bindings: json!({}),
        coding,
        ready: false,
        git: None,
        private_state_writable: true,
        confinement_proven: true,
        native_provider_tool_loop: None,
        check_programs_installed: true,
        extensions: Vec::new(),
        extension_protocol_runtime_health: Vec::new(),
        blocked: Vec::new(),
    }
}

#[test]
fn the_report_keeps_its_published_key_set_and_token_admission_object() {
    let value = report(facts(ResolvedCoding::default()));
    let keys: BTreeSet<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let expected: &[&str] = &[
        "blocked",
        "check_programs_installed",
        "confinement_proven",
        "database_required",
        "enabled",
        "extension_protocol_runtime_health",
        "extensions",
        "git",
        "limits",
        "native_provider_tool_loop",
        "private_state_writable",
        "provider_bindings",
        "ready",
        "token_admission",
        "token_ceilings",
    ];
    assert_eq!(keys, expected.iter().copied().collect());
    let admission = value["token_admission"].clone();
    assert_eq!(
        admission,
        json!({"configured":false,"mode":"independent_reported_thresholds_pending_shared_owner"}),
        "token_admission keeps its published shape"
    );
    assert_eq!(admission.as_object().unwrap().len(), 2);
}

#[test]
fn token_ceilings_disclose_the_per_loop_limit_and_the_default_off_job_total() {
    let ceilings = report(facts(ResolvedCoding::default()))["token_ceilings"].clone();
    assert_eq!(ceilings.as_object().unwrap().len(), 2);
    let per_loop = ceilings["per_loop_reported_tokens"].clone();
    assert_eq!(per_loop["limit"], json!(400_000));
    assert_eq!(per_loop["enforced"], json!(true));
    assert_eq!(per_loop["basis"], "provider_reported_usage");
    assert_eq!(
        per_loop["note"],
        "each model loop stops once its own reported tokens reach the limit; one in-flight request can overshoot"
    );
    let job_total = ceilings["job_total"].clone();
    assert_eq!(job_total["limit"], Value::Null);
    assert_eq!(job_total["enforced"], json!(false));
    assert_eq!(job_total["default"], "not enforced");
    assert_eq!(
        job_total["note"],
        "not enforced unless coding.max_job_tokens is set"
    );
}

#[test]
fn a_configured_job_total_is_disclosed_with_its_number_but_still_not_enforced() {
    let coding = ResolvedCoding {
        max_job_tokens: Some(50_000),
        ..Default::default()
    };
    let job_total = report(facts(coding))["token_ceilings"]["job_total"].clone();
    assert_eq!(job_total["limit"], json!(50_000));
    assert_eq!(job_total["enforced"], json!(false));
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
