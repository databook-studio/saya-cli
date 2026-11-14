//! The one-shot consent, id and result contract, stated without a run.

use super::*;
use saya_agent::ApprovalPolicy;
use saya_types::{CodingFailureClass, CodingRunFailure, JobId, JobState};

fn mode(value: &str) -> OneShotMode {
    resolve_mode(Some(value), None, false).expect("a stated mode resolves")
}

#[test]
fn unstated_mode_is_read_only_with_no_write_scope() {
    assert_eq!(resolve_mode(None, None, false), Ok(OneShotMode::ReadOnly));
    let plan = launch_plan(OneShotMode::ReadOnly, &[], false).expect("read-only plans");
    assert_eq!(plan.policy, ApprovalPolicy::ReadOnly);
    assert!(plan.write_paths.is_empty());
}

#[test]
fn accept_edits_defaults_the_write_scope_to_the_folder() {
    assert_eq!(mode("acceptEdits"), OneShotMode::AcceptEdits);
    assert_eq!(mode("accept-edits"), OneShotMode::AcceptEdits);
    let plan = launch_plan(OneShotMode::AcceptEdits, &[], false).expect("accept-edits plans");
    assert_eq!(plan.policy, ApprovalPolicy::AcceptEdits);
    assert_eq!(plan.write_paths, vec![".".to_owned()]);
}

#[test]
fn stated_write_paths_replace_the_default() {
    let plan = launch_plan(OneShotMode::AcceptEdits, &["src".to_owned()], false)
        .expect("stated paths plan");
    assert_eq!(plan.write_paths, vec!["src".to_owned()]);
    let plan = launch_plan(OneShotMode::Bypass, &["src".to_owned()], false).expect("bypass plans");
    assert_eq!(plan.write_paths, vec!["src".to_owned()]);
}

#[test]
fn write_without_an_edit_mode_is_refused() {
    for mode in [OneShotMode::ReadOnly, OneShotMode::Never] {
        let error = launch_plan(mode, &["src".to_owned()], false)
            .expect_err("a write scope needs an edit-capable mode");
        assert!(
            error.contains("--permission-mode accept-edits"),
            "the refusal names the fix: {error}"
        );
    }
}

#[test]
fn permission_mode_bypass_is_ungated_on_print() {
    assert_eq!(
        resolve_mode(Some("bypass"), None, false),
        Ok(OneShotMode::Bypass)
    );
    assert_eq!(
        resolve_mode(Some("bypassPermissions"), None, false),
        Ok(OneShotMode::Bypass)
    );
    assert_eq!(
        resolve_mode(None, Some("bypass"), false),
        Ok(OneShotMode::Bypass)
    );
}

#[test]
fn plan_mode_is_refused() {
    assert!(resolve_mode(Some("plan"), None, false).is_err());
    assert!(resolve_mode(None, Some("plan"), false).is_err());
}

#[test]
fn generated_job_ids_parse_and_sort_by_time() {
    let first = generate_job_id();
    assert!(first.as_str().starts_with('p'), "{first}");
    assert!(JobId::parse(first.as_str()).is_ok());
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second = generate_job_id();
    assert!(first < second, "{first} sorts before {second}");
}

#[test]
fn exit_codes_follow_the_state() {
    assert_eq!(super::oneshot_result::exit_code(JobState::Completed), 0);
    assert_eq!(super::oneshot_result::exit_code(JobState::Failed), 5);
    assert_eq!(super::oneshot_result::exit_code(JobState::Paused), 6);
    assert_eq!(super::oneshot_result::exit_code(JobState::Cancelled), 130);
}

#[test]
fn subtype_names_cost_and_turn_stops() {
    assert_eq!(super::oneshot_result::subtype(None), "success");
    let cost = CodingRunFailure::root(CodingFailureClass::Cost);
    assert_eq!(
        super::oneshot_result::subtype(Some(&cost)),
        "error_max_budget_usd"
    );
    let turns = CodingRunFailure::root(CodingFailureClass::Backstop).with_guard(
        "limits.job.logical_requests",
        Some(1),
        Some(1),
        "stopped: the limits.job.logical_requests backstop was reached",
    );
    assert_eq!(
        super::oneshot_result::subtype(Some(&turns)),
        "error_max_turns"
    );
    let other = CodingRunFailure::root(CodingFailureClass::ProviderError);
    assert_eq!(
        super::oneshot_result::subtype(Some(&other)),
        "error_during_execution"
    );
}
