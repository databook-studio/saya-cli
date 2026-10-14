use std::num::NonZeroU64;

use saya_types::{
    ApprovalRequestContext, ApprovalRequestError, ApprovalRequestV1, ApprovalScope, JobId,
    ManagedCheckoutId, Sha256Digest,
};
use serde_json::{Value, json};

const POLICY_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const ROOT_PENDING: &str = r#"{"version":1,"job_id":"job-7","agent_id":null,"invocation_id":null,"request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"workspace_write","managed_checkout_id":"checkout-1","relative_path":"src/lib.rs"}}"#;

fn positive(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn workspace_scope() -> ApprovalScope {
    ApprovalScope::workspace_write(
        ManagedCheckoutId::parse("checkout-1").unwrap(),
        "src/lib.rs",
    )
    .unwrap()
}

fn assert_invalid_actor(value: Value, expected: ApprovalRequestError) {
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(ApprovalRequestV1::from_json_bytes(&bytes), Err(expected));
    assert!(serde_json::from_value::<ApprovalRequestV1>(value).is_err());
}

#[test]
fn coordinator_request_uses_explicit_null_actor_and_preserves_child_wire() {
    let context =
        ApprovalRequestContext::new(Sha256Digest::parse(POLICY_SHA).unwrap(), 0, 0, 5000).unwrap();
    let coordinator = ApprovalRequestV1::new_for_coordinator(
        JobId::parse("job-7").unwrap(),
        positive(19),
        context,
        workspace_scope(),
    )
    .unwrap();
    let actual: Value = serde_json::from_slice(&coordinator.to_json_bytes().unwrap()).unwrap();
    assert_eq!(actual, serde_json::from_str::<Value>(ROOT_PENDING).unwrap());

    let decoded = ApprovalRequestV1::from_json_bytes(ROOT_PENDING.as_bytes()).unwrap();
    assert!(decoded.agent_id().is_none());
    assert!(decoded.invocation_id().is_none());
}

#[test]
fn nullable_actor_fields_must_both_be_present_and_both_match() {
    for missing in ["agent_id", "invocation_id"] {
        let mut value: Value = serde_json::from_str(ROOT_PENDING).unwrap();
        value.as_object_mut().unwrap().remove(missing);
        assert_invalid_actor(value, ApprovalRequestError::Malformed);
    }

    let mut coordinator_with_invocation: Value = serde_json::from_str(ROOT_PENDING).unwrap();
    coordinator_with_invocation["invocation_id"] = json!("inv-4");
    assert_invalid_actor(
        coordinator_with_invocation,
        ApprovalRequestError::InvalidActor,
    );

    let mut agent_without_invocation: Value = serde_json::from_str(ROOT_PENDING).unwrap();
    agent_without_invocation["agent_id"] = json!("agent-2");
    assert_invalid_actor(agent_without_invocation, ApprovalRequestError::InvalidActor);
}
