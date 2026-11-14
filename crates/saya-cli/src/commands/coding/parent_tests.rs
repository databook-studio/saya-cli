use super::{CodingError, map_root_error};
use crate::commands::coding::limits::{
    agent_limits_for as agent_limits, answering_limits_for as answering_limits,
};
use saya_agent::{AgentError, ProviderError};
use saya_config::{BudgetLimit, CodingBudgetMode, ResolvedJobLimits};
use std::time::Duration;

fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

/// The `[limits.job]` dimensions, as `coding doctor` states them.
fn stated() -> ResolvedJobLimits {
    ResolvedJobLimits {
        budget_mode: CodingBudgetMode::Bounded,
        logical_requests: finite(45),
        transport_attempts: finite(180),
        tool_calls: finite(60),
        known_tokens: finite(70_000),
        job_tokens: finite(600_000),
        elapsed_seconds: finite(900),
        ..ResolvedJobLimits::default()
    }
}

#[test]
fn the_root_and_helper_ceilings_are_the_resolved_coding_dimensions() {
    let limits = stated();
    let root = agent_limits(&limits, 4096);
    assert_eq!(
        root.max_turns,
        Some(45),
        "the root loop takes the resolved logical requests"
    );
    assert_eq!(root.max_tool_calls, Some(60));
    assert_eq!(root.context_byte_budget, 4096);
    let helper = answering_limits(&limits);
    assert_eq!(
        helper.max_requests,
        Some(45),
        "a helper is never wider than its root"
    );
    assert_eq!(helper.max_tool_calls, Some(60));
    assert_eq!(helper.max_reported_tokens, Some(70_000));
}

#[test]
fn uncapped_mode_leaves_the_root_and_helper_loops_without_a_ceiling() {
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
    let root = agent_limits(&limits, 4096);
    assert_eq!(root.max_turns, None);
    assert_eq!(root.max_tool_calls, None);
    assert_eq!(
        root.max_continuations,
        Some(3),
        "the loop's own shape stays"
    );
    let helper = answering_limits(&limits);
    assert_eq!(helper.max_requests, None);
    assert_eq!(helper.max_tool_calls, None);
    assert_eq!(helper.max_reported_tokens, None);
}

/// No config can state a dimension beside `uncapped` — resolution refuses it —
/// so this hand-built value is the one way to hold both. The mode is the
/// answer: an adapter that read the field without asking it would bound a job
/// the operator declared uncapped.
#[test]
fn a_stated_dimension_beside_uncapped_mode_is_not_a_ceiling() {
    let limits = ResolvedJobLimits {
        budget_mode: CodingBudgetMode::Uncapped,
        ..stated()
    };
    assert_eq!(agent_limits(&limits, 4096).max_turns, None);
    assert_eq!(answering_limits(&limits).max_reported_tokens, None);
}

/// An unattended job is not a reason to invent a wall-clock ceiling: with no
/// configured value the progress guards are the whole clock policy, and the
/// run continues until it makes progress, answers, or trips `no_progress_turns`.
#[test]
fn an_unattended_job_with_no_configured_wall_clock_has_no_wall_clock_guard() {
    let limits = ResolvedJobLimits::default();
    let root = agent_limits(&limits, 4096);
    let progress = root.progress.expect("the progress guards are configured");
    assert_eq!(
        progress.wall_clock, None,
        "an unset wall clock is no wall clock, attended or not"
    );
    assert_eq!(progress.no_progress_turns, Some(8));
    assert_eq!(progress.repeat_limit, Some(3));
}

/// A configured ceiling is still honoured exactly, unattended included.
#[test]
fn a_configured_wall_clock_is_honoured_unattended() {
    let limits = ResolvedJobLimits {
        wall_clock_seconds: finite(900),
        ..ResolvedJobLimits::default()
    };
    let root = agent_limits(&limits, 4096);
    assert_eq!(
        root.progress.and_then(|progress| progress.wall_clock),
        Some(Duration::from_secs(900))
    );
}

#[test]
fn a_whole_limit_truncation_maps_to_the_output_limit_error_naming_the_configured_number() {
    let error = AgentError::Provider(ProviderError::output_truncated(
        "a partial answer that never completes".into(),
        Vec::new(),
    ));
    let mapped = map_root_error(error, Some(4096), None);
    assert!(
        matches!(
            mapped,
            CodingError::OutputLimit {
                configured: Some(4096)
            }
        ),
        "truncation must map to the typed output-limit error carrying the configured number: {mapped}"
    );
    assert!(
        mapped.to_string().contains("ai.max_output_tokens = 4096"),
        "the message names only the configured limit and its config key: {mapped}"
    );
    let mapped = map_root_error(
        AgentError::Provider(ProviderError::output_truncated(String::new(), Vec::new())),
        Some(2048),
        None,
    );
    assert!(
        matches!(
            mapped,
            CodingError::OutputLimit {
                configured: Some(2048)
            }
        ),
        "the configured limit travels with the error: {mapped}"
    );
}

/// With no configured ceiling the model's own limit was hit, so the message
/// must not name a number the operator never set.
#[test]
fn an_unset_limit_truncation_names_no_configured_number() {
    let error = AgentError::Provider(ProviderError::output_truncated(
        "a partial answer that never completes".into(),
        Vec::new(),
    ));
    let mapped = map_root_error(error, None, None);
    assert!(
        matches!(mapped, CodingError::OutputLimit { configured: None }),
        "the absent limit travels with the error: {mapped}"
    );
    assert!(
        !mapped.to_string().contains("ai.max_output_tokens ="),
        "an unset limit names no configured number: {mapped}"
    );
}

#[test]
fn root_agent_errors_preserve_safe_categories_without_provider_detail() {
    let cases = vec![
        (
            AgentError::Provider(ProviderError::Request("SECRET_PROVIDER_SENTINEL".into())),
            saya_types::CodingFailureClass::ProviderError,
        ),
        (
            AgentError::Provider(ProviderError::InvalidResponse),
            saya_types::CodingFailureClass::ProviderError,
        ),
        (
            AgentError::Provider(ProviderError::Configuration(
                "SECRET_PROVIDER_SENTINEL".into(),
            )),
            saya_types::CodingFailureClass::ProviderError,
        ),
        (
            AgentError::Limit("answer bound reached"),
            saya_types::CodingFailureClass::LimitReached,
        ),
        (
            AgentError::InvalidToolCall,
            saya_types::CodingFailureClass::InvalidToolCall,
        ),
        (
            AgentError::Cancelled,
            saya_types::CodingFailureClass::Cancelled,
        ),
    ];
    for (error, class) in cases {
        let mapped = map_root_error(error, Some(4096), None);
        assert!(
            matches!(&mapped, CodingError::Failed(failure) if failure.class == class && failure.agent_id.is_none() && failure.invocation_id.is_none()),
            "a root failure must retain its safe category: {mapped}"
        );
        assert!(!mapped.to_string().contains("SECRET_PROVIDER_SENTINEL"));
    }
}
