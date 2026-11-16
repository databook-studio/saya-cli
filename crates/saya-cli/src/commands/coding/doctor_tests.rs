use super::{
    DoctorFacts, ProbeOutcome, add_tool_loop_failure, report, tool_loop_blocked,
    tool_loop_failure_field,
};
use saya_config::{BudgetLimit, CodingBudgetMode, ResolvedJobLimits};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn probe(completed: bool, truncated: bool) -> Option<ProbeOutcome> {
    Some(ProbeOutcome {
        completed,
        truncated_by_output_limit: truncated,
    })
}

fn facts(job_limits: ResolvedJobLimits) -> DoctorFacts {
    DoctorFacts {
        provider_bindings: json!({}),
        job_limits,
        agents: saya_config::ResolvedAgents::default(),
        ready: false,
        git: None,
        git_compatibility: None,
        private_state_writable: true,
        confinement_proven: true,
        native_provider_tool_loop: None,
        extensions: Vec::new(),
        extension_protocol_runtime_health: Vec::new(),
        in_place: None,
        blocked: Vec::new(),
        optional: Vec::new(),
    }
}

#[test]
fn the_report_keeps_its_published_key_set_and_token_admission_object() {
    let value = report(facts(ResolvedJobLimits::default()));
    let keys: BTreeSet<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let expected: &[&str] = &[
        "blocked",
        "budget_mode",
        "confinement_proven",
        "database_required",
        "extension_protocol_runtime_health",
        "extensions",
        "git",
        "git_compatibility",
        "limits",
        "native_provider_tool_loop",
        "operational_bounds",
        "optional",
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
fn uncapped_report_discloses_absent_ceilings_and_retained_operational_bounds() {
    let limits = ResolvedJobLimits {
        budget_mode: CodingBudgetMode::Uncapped,
        logical_requests: BudgetLimit::Unlimited,
        transport_attempts: BudgetLimit::Unlimited,
        tool_calls: BudgetLimit::Unlimited,
        known_tokens: BudgetLimit::Unlimited,
        job_tokens: BudgetLimit::Unlimited,
        elapsed_seconds: BudgetLimit::Unlimited,
        ..ResolvedJobLimits::default()
    };
    let value = report(facts(limits));
    assert_eq!(value["budget_mode"], "uncapped");
    for field in [
        "max_logical_model_requests",
        "max_physical_transport_attempts",
        "max_tool_calls",
        "max_known_tokens",
        "max_elapsed_seconds",
    ] {
        assert_eq!(value["limits"][field], Value::Null, "{field}");
    }
    assert_eq!(
        value["token_ceilings"]["per_loop_reported_tokens"]["enforced"],
        json!(false)
    );
    assert_eq!(
        value["token_ceilings"]["job_total"]["enforced"],
        json!(false)
    );
    assert_eq!(
        value["operational_bounds"]["max_reservations_per_job"],
        10_000
    );
    assert_eq!(value["operational_bounds"]["max_live_permits"], 1_024);
    assert_eq!(value["operational_bounds"]["max_command_slots"], 2);
    assert_eq!(
        value["operational_bounds"]["max_resume_history_events"],
        65_536
    );
}

#[test]
fn token_ceilings_disclose_no_default_per_loop_limit_and_the_default_off_job_total() {
    let ceilings = report(facts(ResolvedJobLimits::default()))["token_ceilings"].clone();
    assert_eq!(ceilings.as_object().unwrap().len(), 2);
    let per_loop = ceilings["per_loop_reported_tokens"].clone();
    assert_eq!(per_loop["limit"], Value::Null);
    assert_eq!(per_loop["enforced"], json!(false));
    assert_eq!(per_loop["basis"], "provider_reported_usage");
    assert_eq!(
        per_loop["note"],
        "not enforced unless limits.job.known_tokens is set"
    );
    let job_total = ceilings["job_total"].clone();
    assert_eq!(job_total["limit"], Value::Null);
    assert_eq!(job_total["enforced"], json!(false));
    assert_eq!(job_total["default"], "not enforced");
    assert_eq!(
        job_total["note"],
        "not enforced unless limits.job.job_tokens is set"
    );
}

#[test]
fn a_configured_job_total_is_disclosed_with_its_number_and_enforced() {
    let limits = ResolvedJobLimits {
        job_tokens: BudgetLimit::Finite(std::num::NonZeroU64::new(50_000).unwrap()),
        ..Default::default()
    };
    let job_total = report(facts(limits))["token_ceilings"]["job_total"].clone();
    assert_eq!(job_total["limit"], json!(50_000));
    assert_eq!(job_total["enforced"], json!(true));
    let note = job_total["note"]
        .as_str()
        .expect("the configured job ceiling must explain its enforcement basis")
        .to_ascii_lowercase();
    assert!(
        note.contains("only") && note.contains("provider-reported usage"),
        "the configured ceiling must disclose that only provider-reported usage counts: {note}"
    );
    assert!(
        note.contains("concurrent requests")
            && note.contains("already admitted")
            && note.contains("overshoot"),
        "the configured ceiling must disclose already-admitted concurrent requests may overshoot: {note}"
    );
}

#[test]
fn a_truncated_probe_adds_the_output_limit_failure_field_naming_the_configured_number() {
    let field = tool_loop_failure_field(probe(false, true), Some(4096))
        .expect("a probe the output limit cut off must carry the failure field");
    assert_eq!(field["reason"], "output_limit");
    assert_eq!(field["configured_max_output_tokens"], 4096);
    assert_eq!(
        field["action"],
        "raise ai.max_output_tokens in trusted configuration"
    );
    let field = tool_loop_failure_field(probe(false, true), Some(8192))
        .expect("the failure field is built for the configured limit, whatever it is");
    assert_eq!(field["configured_max_output_tokens"], 8192);
}

#[test]
fn a_healthy_or_differently_failed_probe_builds_no_failure_field() {
    assert!(tool_loop_failure_field(probe(true, false), Some(4096)).is_none());
    assert!(tool_loop_failure_field(probe(false, false), Some(4096)).is_none());
    assert!(tool_loop_failure_field(None, Some(4096)).is_none());
}

#[test]
fn the_doctor_json_carries_the_failure_field_only_for_a_truncated_probe() {
    let truncated = add_tool_loop_failure(
        json!({"ready": false}),
        tool_loop_failure_field(probe(false, true), Some(4096)),
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
        tool_loop_failure_field(probe(true, false), Some(4096)),
    );
    assert!(
        healthy.get("native_provider_tool_loop_failure").is_none(),
        "a healthy probe must not carry the field: {healthy}"
    );
    let other = add_tool_loop_failure(
        json!({"ready": false}),
        tool_loop_failure_field(probe(false, false), Some(4096)),
    );
    assert!(
        other.get("native_provider_tool_loop_failure").is_none(),
        "a probe that failed for another reason must not carry the field: {other}"
    );
}

#[test]
fn a_truncated_probe_blocks_with_the_output_limit_action_instead_of_the_generic_text() {
    let entry = tool_loop_blocked(probe(false, true), Some(4096))
        .expect("a probe the output limit cut off must block readiness");
    assert!(entry.contains("output-token limit"), "{entry}");
    assert!(entry.contains("ai.max_output_tokens = 4096"), "{entry}");
    assert!(
        entry.contains("raise ai.max_output_tokens in trusted configuration"),
        "{entry}"
    );
    assert!(entry.contains("saya job doctor --probe"), "{entry}");
}

#[test]
fn a_healthy_probe_blocks_nothing_and_every_other_case_keeps_the_generic_text() {
    assert_eq!(tool_loop_blocked(probe(true, false), Some(4096)), None);
    let generic = "Run saya job doctor --probe and verify the configured provider completes the native tool loop";
    assert_eq!(
        tool_loop_blocked(None, Some(4096)).as_deref(),
        Some(generic)
    );
    assert_eq!(
        tool_loop_blocked(probe(false, false), Some(4096)).as_deref(),
        Some(generic)
    );
}

#[test]
fn the_report_carries_the_detected_git_compatibility() {
    let mut facts = facts(ResolvedJobLimits::default());
    facts.git_compatibility = Some(json!({
        "version": "2.34.1",
        "version_known": true,
        "version_source": "reported",
        "minimum_supported": "2.34.0",
        "lazy_fetch_protection": "refuse_partial_clone",
        "worktree_list_zero_terminated": false,
    }));
    let value = report(facts);
    assert_eq!(value["git_compatibility"]["version"], "2.34.1");
    assert_eq!(value["git_compatibility"]["version_source"], "reported");
    assert_eq!(
        value["git_compatibility"]["lazy_fetch_protection"],
        "refuse_partial_clone"
    );
}
