use super::{BUDGET_POLICY_V2_VERSION, BudgetLimitPolicyV2};
use crate::coding::{BudgetLimitPolicyV1, BudgetPolicyError, CodingBudgetMode, CodingBudgetPolicy};

fn policy() -> BudgetLimitPolicyV2 {
    BudgetLimitPolicyV2::uncapped(4, 4).unwrap()
}

#[test]
fn uncapped_policy_has_explicit_no_caps_and_finite_concurrency() {
    let policy = policy();
    assert_eq!(policy.version(), BUDGET_POLICY_V2_VERSION);
    assert_eq!(policy.budget_mode(), CodingBudgetMode::Uncapped);
    assert_eq!(policy.logical_requests(), None);
    assert_eq!(policy.transport_attempts(), None);
    assert_eq!(policy.tool_calls(), None);
    assert_eq!(policy.per_loop_tokens(), None);
    assert_eq!(policy.job_tokens(), None);
    assert_eq!(policy.elapsed_millis(), None);
    assert_eq!(policy.model_slots(), 4);
    assert_eq!(policy.child_slots(), 4);

    let bytes = policy.canonical_json_bytes().unwrap();
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        r#"{"version":2,"budget_mode":"uncapped","logical_requests":null,"transport_attempts":null,"tool_calls":null,"per_loop_tokens":null,"job_tokens":null,"elapsed_millis":null,"model_slots":4,"child_slots":4}"#
    );
    assert_eq!(
        BudgetLimitPolicyV2::from_json_bytes(&bytes).unwrap(),
        policy
    );
    assert_eq!(
        CodingBudgetPolicy::from_json_bytes(&bytes).unwrap(),
        CodingBudgetPolicy::UncappedV2(policy)
    );
}

#[test]
fn uncapped_policy_rejects_explicit_caps_wrong_mode_and_noncanonical_missing_fields() {
    let canonical = std::str::from_utf8(&policy().canonical_json_bytes().unwrap())
        .unwrap()
        .to_owned();
    let capped = canonical.replace("\"logical_requests\":null", "\"logical_requests\":1");
    assert_eq!(
        BudgetLimitPolicyV2::from_json_bytes(capped.as_bytes()).unwrap_err(),
        BudgetPolicyError::InvalidUncappedPolicy
    );
    let bounded = canonical.replace("\"uncapped\"", "\"bounded\"");
    assert_eq!(
        BudgetLimitPolicyV2::from_json_bytes(bounded.as_bytes()).unwrap_err(),
        BudgetPolicyError::InvalidUncappedPolicy
    );
    let missing = canonical.replace("\"job_tokens\":null,", "");
    assert_eq!(
        BudgetLimitPolicyV2::from_json_bytes(missing.as_bytes()).unwrap_err(),
        BudgetPolicyError::Malformed
    );
    let unknown = canonical.replace("\"child_slots\":4", "\"child_slots\":4,\"cost\":0");
    assert_eq!(
        BudgetLimitPolicyV2::from_json_bytes(unknown.as_bytes()).unwrap_err(),
        BudgetPolicyError::Malformed
    );
    let future_version = canonical.replace("\"version\":2", "\"version\":3");
    assert_eq!(
        CodingBudgetPolicy::from_json_bytes(future_version.as_bytes()).unwrap_err(),
        BudgetPolicyError::UnsupportedVersion(3)
    );
}

#[test]
fn version_dispatch_preserves_bounded_v1_policy_bytes() {
    let bounded = BudgetLimitPolicyV1::new(120, 240, 80, 4, 2, 1_800_000).unwrap();
    let bytes = bounded.canonical_json_bytes().unwrap();
    assert_eq!(
        CodingBudgetPolicy::from_json_bytes(&bytes).unwrap(),
        CodingBudgetPolicy::BoundedV1(bounded.clone())
    );
    assert_eq!(
        CodingBudgetPolicy::BoundedV1(bounded.clone())
            .canonical_json_bytes()
            .unwrap(),
        bytes
    );
    assert_eq!(
        CodingBudgetPolicy::BoundedV1(bounded),
        BudgetLimitPolicyV1::new(120, 240, 80, 4, 2, 1_800_000).unwrap()
    );
}

#[test]
fn uncapped_policy_requires_positive_finite_concurrency() {
    assert_eq!(
        BudgetLimitPolicyV2::uncapped(0, 1).unwrap_err(),
        BudgetPolicyError::ZeroLimit
    );
    assert_eq!(
        BudgetLimitPolicyV2::uncapped(1, 0).unwrap_err(),
        BudgetPolicyError::ZeroLimit
    );
}
