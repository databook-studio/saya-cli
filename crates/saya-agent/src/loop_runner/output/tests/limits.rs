use super::super::*;

#[test]
fn default_limits_are_unbounded() {
    let limits = AgentLimits::default();
    assert!(limits.max_turns.is_none(), "no turn ceiling by default");
    assert!(
        limits.max_tool_calls.is_none(),
        "no tool-call ceiling by default"
    );
    assert!(
        !limits.permit_candidate_writes && !limits.permit_workspace_writes,
        "write permits default off"
    );
    assert!(
        !limits.permit_external_effects,
        "external effects default off: the guard keeps denying exactly as before, so \
             interactive turns are byte-identical"
    );
}

#[test]
fn default_continuation_ceiling_is_three() {
    assert_eq!(
        AgentLimits::default().max_continuations,
        Some(3),
        "continuation defaults bounded: an operator who never heard of \
             the variable still gets the bound"
    );
}

#[test]
fn continuation_budget_unset_means_default_bound() {
    let budgets = budgets_from_env(|_| None);
    assert_eq!(budgets.max_continuations, Some(3));
}

#[test]
fn continuation_budget_zero_disables_continuation() {
    let budgets = budgets_from_env(|name| match name {
        "SAYA_AGENT_MAX_CONTINUATIONS" => Some("0".to_string()),
        _ => None,
    });
    assert_eq!(budgets.max_continuations, Some(0));
}

#[test]
fn continuation_budget_positive_value_is_honored() {
    let budgets = budgets_from_env(|name| match name {
        "SAYA_AGENT_MAX_CONTINUATIONS" => Some("7".to_string()),
        _ => None,
    });
    assert_eq!(budgets.max_continuations, Some(7));
}

#[test]
fn continuation_budget_unparseable_means_default_bound_never_unbounded() {
    for raw in ["abc", "", "-1"] {
        let budgets = budgets_from_env(|name| match name {
            "SAYA_AGENT_MAX_CONTINUATIONS" => Some(raw.to_string()),
            _ => None,
        });
        assert_eq!(
            budgets.max_continuations,
            Some(3),
            "unparseable {raw:?} must fall back to the bound, never unbounded"
        );
    }
}

#[test]
fn turn_and_tool_call_budgets_stay_unbounded_when_unset_through_new_struct() {
    let budgets = budgets_from_env(|_| None);
    assert!(budgets.max_turns.is_none());
    assert!(budgets.max_tool_calls.is_none());
}

/// All three ceiling variables set at once parse independently onto their
/// own field. The composition onto `AgentLimits` — which value the real
/// runtime puts on which limit — is proven at the runtime owner
/// (`crates/saya-cli/src/agent/runtime_budget_tests.rs`); asserting it here
/// only re-checked this test's own copy of the assignment.
#[test]
fn env_budgets_parse_each_ceiling_when_all_three_are_set_at_once() {
    let lookup = |name: &str| match name {
        "SAYA_AGENT_MAX_TURNS" => Some("11".to_string()),
        "SAYA_AGENT_MAX_TOOL_CALLS" => Some("13".to_string()),
        "SAYA_AGENT_MAX_CONTINUATIONS" => Some("5".to_string()),
        _ => None,
    };
    let budgets = budgets_from_env(lookup);
    assert_eq!(budgets.max_turns, Some(11));
    assert_eq!(budgets.max_tool_calls, Some(13));
    assert_eq!(budgets.max_continuations, Some(5));
}

#[test]
fn budgets_from_env_sets_each_ceiling_independently_with_no_upper_limit() {
    let lookup = |name: &str| match name {
        "SAYA_AGENT_MAX_TURNS" => Some("1000000".to_string()),
        "SAYA_AGENT_MAX_TOOL_CALLS" => Some("7".to_string()),
        _ => None,
    };
    let budgets = budgets_from_env(lookup);
    assert_eq!(budgets.max_turns, Some(1_000_000));
    assert_eq!(budgets.max_tool_calls, Some(7));
}

#[test]
fn budgets_from_env_is_unbounded_when_unset_or_unparseable() {
    let budgets = budgets_from_env(|_| None);
    assert!(budgets.max_turns.is_none() && budgets.max_tool_calls.is_none());
    // A set but unparseable value is treated as unset, not as zero: a typo
    // cannot silently collapse the ceiling to the smallest bound.
    let lookup = |name: &str| match name {
        "SAYA_AGENT_MAX_TURNS" => Some("not-a-number".to_string()),
        _ => None,
    };
    let budgets = budgets_from_env(lookup);
    assert!(budgets.max_turns.is_none());
}
