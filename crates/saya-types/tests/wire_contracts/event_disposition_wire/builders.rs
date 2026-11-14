use std::num::NonZeroU64;

use saya_types::{
    AgentId, ApprovalRequestContext, ApprovalRequestV1, ApprovalScope, BudgetCharge,
    BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CheckoutOperationState, CheckoutOperationV1,
    CodingEventKind, CodingEventV1, GitObjectId, JobId, ManagedCheckoutId,
    ManagedCheckoutLifecycle, ManagedCheckoutPath, ManagedCheckoutRecordData,
    ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction, Sha256Digest, SnapshotId,
};
use serde_json::Value;

const SOURCE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMMON_SHA: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const SNAPSHOT_SHA: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const BASE_COMMIT: &str = "1111111111111111111111111111111111111111";

pub(super) fn positive(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

pub(super) fn job(value: &str) -> JobId {
    JobId::parse(value).unwrap()
}

pub(super) fn event(
    sequence: u64,
    job_id: JobId,
    agent_id: Option<AgentId>,
    invocation_id: Option<saya_types::InvocationId>,
    event: CodingEventKind,
) -> CodingEventV1 {
    CodingEventV1::new(positive(sequence), job_id, agent_id, invocation_id, event).unwrap()
}

pub(super) fn assert_event_matches(event: CodingEventV1, expected: &str) {
    let bytes = event.to_json_bytes().unwrap();
    assert_json_matches(&bytes, expected);
    let decoded = CodingEventV1::from_json_bytes(expected.as_bytes()).unwrap();
    assert_json_matches(&decoded.to_json_bytes().unwrap(), expected);
}

pub(super) fn assert_json_matches(actual: &[u8], expected: &str) {
    let actual: Value = serde_json::from_slice(actual).unwrap();
    let expected: Value = serde_json::from_str(expected).unwrap();
    assert_eq!(actual, expected);
}

pub(super) fn budget_reservation() -> BudgetReservationV1 {
    BudgetReservationV1::new(
        job("job-1"),
        positive(5),
        Some(AgentId::parse("agent-1").unwrap()),
        Some(saya_types::InvocationId::parse("inv-1").unwrap()),
        vec![BudgetCharge::new(BudgetDimension::LogicalRequests, None, None, positive(1)).unwrap()],
    )
    .unwrap()
}

pub(super) fn budget_settlement() -> BudgetSettlementV1 {
    BudgetSettlementV1::new(
        job("job-1"),
        positive(5),
        positive(6),
        BudgetEffectState::Uncertain,
        vec![
            BudgetObservation::new(
                BudgetDimension::LogicalRequests,
                None,
                None,
                BudgetObservationValue::Unknown,
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

pub(super) fn approval_request() -> ApprovalRequestV1 {
    ApprovalRequestV1::new_for_coordinator(
        job("job-1"),
        positive(7),
        ApprovalRequestContext::new(Sha256Digest::parse(SOURCE_SHA).unwrap(), 0, 0, 5000).unwrap(),
        ApprovalScope::workspace_write(
            ManagedCheckoutId::parse("checkout-1").unwrap(),
            "src/lib.rs",
        )
        .unwrap(),
    )
    .unwrap()
}

pub(super) fn checkout_record() -> ManagedCheckoutRecordV1 {
    ManagedCheckoutRecordV1::new(ManagedCheckoutRecordData {
        job_id: job("job-1"),
        agent_id: Some(AgentId::parse("agent-1").unwrap()),
        managed_checkout_id: ManagedCheckoutId::parse("checkout-1").unwrap(),
        repository_id: RepositoryId::parse("repo-shared-1").unwrap(),
        common_dir_sha256: Sha256Digest::parse(COMMON_SHA).unwrap(),
        managed_path: ManagedCheckoutPath::new("/Users/saya/state/checkouts/checkout-1").unwrap(),
        base_commit: GitObjectId::parse(BASE_COMMIT).unwrap(),
        snapshot_id: SnapshotId::parse("snapshot-1").unwrap(),
        snapshot_sha256: Sha256Digest::parse(SNAPSHOT_SHA).unwrap(),
        created_by_sequence: positive(11),
        lifecycle: ManagedCheckoutLifecycle::Creating,
    })
    .unwrap()
}

pub(super) fn checkout_operation(
    sequence: u64,
    action: RepositoryMutationAction,
    state: CheckoutOperationState,
) -> CheckoutOperationV1 {
    CheckoutOperationV1::new(
        job("job-1"),
        positive(sequence),
        ManagedCheckoutId::parse("checkout-1").unwrap(),
        action,
        state,
    )
    .unwrap()
}
