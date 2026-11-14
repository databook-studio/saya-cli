//! Tests for the durable budget policy identity (`saya-types::coding::budget_policy`).

use super::*;

const GOLDEN_POLICY_JSON: &str = r#"{"version":1,"logical_requests":120,"transport_attempts":240,"tool_calls":80,"model_slots":4,"child_slots":2,"deadline_millis":1800000}"#;
// Independently computed: printf '%s' "$GOLDEN_POLICY_JSON" | shasum -a 256
const GOLDEN_POLICY_SHA256: &str =
    "eae0696dc0ca44e40daa9e8ab8ca26f00e08bfec47737fd8cdee18e27ab190c3";

fn golden_policy() -> BudgetLimitPolicyV1 {
    BudgetLimitPolicyV1::new(120, 240, 80, 4, 2, 1_800_000).unwrap()
}

#[test]
fn canonical_bytes_are_stable_golden() {
    let policy = golden_policy();
    let bytes = policy.canonical_json_bytes().unwrap();
    assert_eq!(
        String::from_utf8(bytes.clone()).unwrap(),
        GOLDEN_POLICY_JSON
    );
    let digest = Sha256Digest::parse(GOLDEN_POLICY_SHA256).unwrap();
    assert_eq!(policy.digest(), digest);
    // An independently constructed equal policy hashes to the same identity.
    let rebuilt = BudgetLimitPolicyV1::new(120, 240, 80, 4, 2, 1_800_000).unwrap();
    assert_eq!(rebuilt.digest(), digest);
    // Any limit change must change the digest.
    let shifted = BudgetLimitPolicyV1::new(121, 240, 80, 4, 2, 1_800_000).unwrap();
    assert_ne!(shifted.digest(), digest);
    assert!(bytes.len() < 65_536);
}

#[test]
fn invalid_limits_are_refused() {
    for limits in [
        (0, 240, 80, 4, 2, 1_800_000),
        (120, 0, 80, 4, 2, 1_800_000),
        (120, 240, 0, 4, 2, 1_800_000),
        (120, 240, 80, 0, 2, 1_800_000),
        (120, 240, 80, 4, 0, 1_800_000),
        (120, 240, 80, 4, 2, 0),
    ] {
        assert_eq!(
            BudgetLimitPolicyV1::new(limits.0, limits.1, limits.2, limits.3, limits.4, limits.5)
                .unwrap_err(),
            BudgetPolicyError::ZeroLimit
        );
    }
    assert!(BudgetLimitPolicyV1::new(1, 1, 1, 1, 1, 1).is_ok());
}

#[test]
fn deserialize_revalidates() {
    let bytes = golden_policy().canonical_json_bytes().unwrap();
    let parsed: BudgetLimitPolicyV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed, golden_policy());
    assert_eq!(parsed.digest(), golden_policy().digest());

    let zero_limit = r#"{"version":1,"logical_requests":0,"transport_attempts":240,"tool_calls":80,"model_slots":4,"child_slots":2,"deadline_millis":1800000}"#;
    assert_eq!(
        BudgetLimitPolicyV1::from_json_bytes(zero_limit.as_bytes()).unwrap_err(),
        BudgetPolicyError::ZeroLimit
    );

    let future_version = r#"{"version":2,"logical_requests":120,"transport_attempts":240,"tool_calls":80,"model_slots":4,"child_slots":2,"deadline_millis":1800000}"#;
    assert_eq!(
        BudgetLimitPolicyV1::from_json_bytes(future_version.as_bytes()).unwrap_err(),
        BudgetPolicyError::UnsupportedVersion(2)
    );

    // The raw serde Deserialize path refuses the same records; its errors are
    // serde_json::Error values, so only the category (Data, not Syntax) is
    // asserted here — the typed reasons are proven through from_json_bytes.
    assert!(
        serde_json::from_slice::<BudgetLimitPolicyV1>(zero_limit.as_bytes())
            .unwrap_err()
            .is_data()
    );
    assert!(
        serde_json::from_slice::<BudgetLimitPolicyV1>(future_version.as_bytes())
            .unwrap_err()
            .is_data()
    );

    let unknown_field = r#"{"version":1,"logical_requests":120,"transport_attempts":240,"tool_calls":80,"model_slots":4,"child_slots":2,"deadline_millis":1800000,"cost_microusd":0}"#;
    assert!(serde_json::from_slice::<BudgetLimitPolicyV1>(unknown_field.as_bytes()).is_err());

    assert_eq!(
        BudgetLimitPolicyV1::from_json_bytes(b"not json").unwrap_err(),
        BudgetPolicyError::Malformed
    );
}
