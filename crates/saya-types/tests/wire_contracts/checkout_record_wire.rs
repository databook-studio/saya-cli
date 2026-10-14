use std::num::NonZeroU64;

use saya_types::{
    AgentId, CheckoutDtoError, CheckoutOperationState, CheckoutOperationV1, GitObjectId, JobId,
    ManagedCheckoutId, ManagedCheckoutLifecycle, ManagedCheckoutPath, ManagedCheckoutRecordData,
    ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction, Sha256Digest, SnapshotId,
};
use serde_json::{Value, json};

const BASE_COMMIT: &str = "1111111111111111111111111111111111111111";
const COMMON_DIR_SHA: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const SNAPSHOT_SHA: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const CHILD_RECORD: &str = r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","managed_checkout_id":"checkout-1","repository_id":"repo-shared-1","common_dir_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","managed_path":"/Users/saya/state/checkouts/checkout-1","base_commit":"1111111111111111111111111111111111111111","snapshot_id":"snapshot-1","snapshot_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","created_by_sequence":11,"lifecycle":"creating"}"#;
const CANDIDATE_RECORD: &str = r#"{"version":1,"job_id":"job-7","agent_id":null,"managed_checkout_id":"candidate-1","repository_id":"repo-shared-1","common_dir_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","managed_path":"/Users/saya/state/checkouts/candidate-1","base_commit":"1111111111111111111111111111111111111111","snapshot_id":"snapshot-1","snapshot_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","created_by_sequence":21,"lifecycle":"creating"}"#;
const CREATE_INTENT: &str = r#"{"version":1,"job_id":"job-7","operation_sequence":11,"managed_checkout_id":"checkout-1","action":"create_checkout","state":"intent"}"#;

fn positive(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn record_data(
    agent_id: Option<AgentId>,
    checkout: &str,
    managed_path: &str,
    created_by_sequence: u64,
    lifecycle: ManagedCheckoutLifecycle,
) -> ManagedCheckoutRecordData {
    ManagedCheckoutRecordData {
        job_id: JobId::parse("job-7").unwrap(),
        agent_id,
        managed_checkout_id: ManagedCheckoutId::parse(checkout).unwrap(),
        repository_id: RepositoryId::parse("repo-shared-1").unwrap(),
        common_dir_sha256: Sha256Digest::parse(COMMON_DIR_SHA).unwrap(),
        managed_path: ManagedCheckoutPath::new(managed_path).unwrap(),
        base_commit: GitObjectId::parse(BASE_COMMIT).unwrap(),
        snapshot_id: SnapshotId::parse("snapshot-1").unwrap(),
        snapshot_sha256: Sha256Digest::parse(SNAPSHOT_SHA).unwrap(),
        created_by_sequence: positive(created_by_sequence),
        lifecycle,
    }
}

fn assert_json_matches(actual: &[u8], expected: &str) {
    let actual: Value = serde_json::from_slice(actual).unwrap();
    let expected: Value = serde_json::from_str(expected).unwrap();
    assert_eq!(actual, expected);
}

fn record_wire(value: Value) -> Result<ManagedCheckoutRecordV1, CheckoutDtoError> {
    ManagedCheckoutRecordV1::from_json_bytes(&serde_json::to_vec(&value).unwrap())
}

fn operation_wire(value: Value) -> Result<CheckoutOperationV1, CheckoutDtoError> {
    CheckoutOperationV1::from_json_bytes(&serde_json::to_vec(&value).unwrap())
}

#[test]
fn checkout_record_and_create_intent_match_fixed_wire_fixtures() {
    let child = ManagedCheckoutRecordV1::new(record_data(
        Some(AgentId::parse("agent-2").unwrap()),
        "checkout-1",
        "/Users/saya/state/checkouts/checkout-1",
        11,
        ManagedCheckoutLifecycle::Creating,
    ))
    .unwrap();
    assert_json_matches(&child.to_json_bytes().unwrap(), CHILD_RECORD);
    let decoded = ManagedCheckoutRecordV1::from_json_bytes(CHILD_RECORD.as_bytes()).unwrap();
    assert_eq!(decoded.job_id().as_str(), "job-7");
    assert_eq!(decoded.agent_id().unwrap().as_str(), "agent-2");
    assert_eq!(decoded.created_by_sequence().get(), 11);

    let candidate = ManagedCheckoutRecordV1::new(record_data(
        None,
        "candidate-1",
        "/Users/saya/state/checkouts/candidate-1",
        21,
        ManagedCheckoutLifecycle::Creating,
    ))
    .unwrap();
    assert_json_matches(&candidate.to_json_bytes().unwrap(), CANDIDATE_RECORD);
    assert!(
        ManagedCheckoutRecordV1::from_json_bytes(CANDIDATE_RECORD.as_bytes())
            .unwrap()
            .agent_id()
            .is_none()
    );

    let operation = CheckoutOperationV1::new(
        JobId::parse("job-7").unwrap(),
        positive(11),
        ManagedCheckoutId::parse("checkout-1").unwrap(),
        RepositoryMutationAction::CreateCheckout,
        CheckoutOperationState::Intent,
    )
    .unwrap();
    assert_json_matches(&operation.to_json_bytes().unwrap(), CREATE_INTENT);
    assert_eq!(
        CheckoutOperationV1::from_json_bytes(CREATE_INTENT.as_bytes())
            .unwrap()
            .operation_sequence()
            .get(),
        decoded.created_by_sequence().get()
    );
}

#[test]
fn checkout_record_rejects_future_unknown_missing_and_zero_fields() {
    let future = CHILD_RECORD.replace("\"version\":1", "\"version\":2");
    assert_eq!(
        ManagedCheckoutRecordV1::from_json_bytes(future.as_bytes()),
        Err(CheckoutDtoError::UnsupportedVersion(2))
    );

    let mut extra: Value = serde_json::from_str(CHILD_RECORD).unwrap();
    extra["permission_grant"] = json!(true);
    assert_eq!(record_wire(extra), Err(CheckoutDtoError::Malformed));

    let mut missing_agent: Value = serde_json::from_str(CHILD_RECORD).unwrap();
    missing_agent.as_object_mut().unwrap().remove("agent_id");
    assert_eq!(record_wire(missing_agent), Err(CheckoutDtoError::Malformed));

    let mut zero_sequence: Value = serde_json::from_str(CHILD_RECORD).unwrap();
    zero_sequence["created_by_sequence"] = json!(0);
    assert_eq!(record_wire(zero_sequence), Err(CheckoutDtoError::Malformed));

    let mut invalid_path: Value = serde_json::from_str(CHILD_RECORD).unwrap();
    invalid_path["managed_path"] = json!("relative/checkout");
    assert_eq!(record_wire(invalid_path), Err(CheckoutDtoError::Malformed));
}

#[test]
fn checkout_wire_preserves_all_lifecycle_action_and_operation_states() {
    let lifecycles = [
        (ManagedCheckoutLifecycle::Creating, "creating"),
        (ManagedCheckoutLifecycle::Active, "active"),
        (ManagedCheckoutLifecycle::Retained, "retained"),
        (ManagedCheckoutLifecycle::Removing, "removing"),
        (ManagedCheckoutLifecycle::Removed, "removed"),
        (ManagedCheckoutLifecycle::Failed, "failed"),
        (ManagedCheckoutLifecycle::Uncertain, "uncertain"),
    ];
    for (lifecycle, expected) in lifecycles {
        let record = ManagedCheckoutRecordV1::new(record_data(
            Some(AgentId::parse("agent-2").unwrap()),
            "checkout-1",
            "/Users/saya/state/checkouts/checkout-1",
            11,
            lifecycle,
        ))
        .unwrap();
        let wire: Value = serde_json::from_slice(&record.to_json_bytes().unwrap()).unwrap();
        assert_eq!(wire["lifecycle"], json!(expected));
    }

    let actions = [
        (RepositoryMutationAction::CreateCheckout, "create_checkout"),
        (RepositoryMutationAction::RemoveCheckout, "remove_checkout"),
        (
            RepositoryMutationAction::IntegrateCandidate,
            "integrate_candidate",
        ),
        (RepositoryMutationAction::ApplyBack, "apply_back"),
    ];
    let states = [
        (CheckoutOperationState::Intent, "intent"),
        (CheckoutOperationState::Completed, "completed"),
        (CheckoutOperationState::Failed, "failed"),
        (CheckoutOperationState::Uncertain, "uncertain"),
    ];
    for (action, action_name) in actions {
        for (state, state_name) in states {
            let operation = CheckoutOperationV1::new(
                JobId::parse("job-7").unwrap(),
                positive(11),
                ManagedCheckoutId::parse("checkout-1").unwrap(),
                action,
                state,
            )
            .unwrap();
            let wire: Value = serde_json::from_slice(&operation.to_json_bytes().unwrap()).unwrap();
            assert_eq!(wire["action"], json!(action_name));
            assert_eq!(wire["state"], json!(state_name));
        }
    }
}

#[test]
fn checkout_operation_keeps_original_intent_key_and_rejects_bad_wire() {
    let mut uncertain: Value = serde_json::from_str(CREATE_INTENT).unwrap();
    uncertain["state"] = json!("uncertain");
    let decoded = operation_wire(uncertain).unwrap();
    assert_eq!(decoded.operation_sequence().get(), 11);
    assert_eq!(decoded.state(), CheckoutOperationState::Uncertain);

    let future = CREATE_INTENT.replace("\"version\":1", "\"version\":2");
    assert_eq!(
        CheckoutOperationV1::from_json_bytes(future.as_bytes()),
        Err(CheckoutDtoError::UnsupportedVersion(2))
    );

    let mut grant: Value = serde_json::from_str(CREATE_INTENT).unwrap();
    grant["grant"] = json!(true);
    assert_eq!(operation_wire(grant), Err(CheckoutDtoError::Malformed));

    let mut zero_sequence: Value = serde_json::from_str(CREATE_INTENT).unwrap();
    zero_sequence["operation_sequence"] = json!(0);
    assert_eq!(
        operation_wire(zero_sequence),
        Err(CheckoutDtoError::Malformed)
    );
}
