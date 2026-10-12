use std::num::NonZeroU64;

use saya_types::{
    AgentId, ApprovalDisposition, ApprovalRequestContext, ApprovalRequestV1, ApprovalScope,
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CheckoutOperationState, CheckoutOperationV1,
    CodingEventError, CodingEventKind, CodingEventV1, GitObjectId, JobId, ManagedCheckoutId,
    ManagedCheckoutLifecycle, ManagedCheckoutPath, ManagedCheckoutRecordData,
    ManagedCheckoutRecordV1, RepositoryId, RepositoryMutationAction, Sha256Digest, SnapshotId,
};
use serde_json::Value;

const SOURCE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COMMON_SHA: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const NOTE_SHA: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const SNAPSHOT_SHA: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const BASE_COMMIT: &str = "1111111111111111111111111111111111111111";

pub(super) const BUDGET_RESERVED: &str = r#"{
  "version": 1, "sequence": 5, "job_id": "job-1", "agent_id": "agent-1",
  "invocation_id": "inv-1", "event": {"type": "budget_reserved", "reservation": {
    "version": 1, "job_id": "job-1", "reservation_sequence": 5,
    "agent_id": "agent-1", "invocation_id": "inv-1", "charges": [{
      "dimension": "logical_requests", "endpoint": null,
      "price_metadata_sha256": null, "reserved": 1
    }]
  }}
}"#;

pub(super) const BUDGET_SETTLED: &str = r#"{
  "version": 1, "sequence": 6, "job_id": "job-1", "event": {
    "type": "budget_settled", "settlement": {
      "version": 1, "job_id": "job-1", "reservation_sequence": 5,
      "settlement_sequence": 6, "effect_state": "uncertain", "observations": [{
        "dimension": "logical_requests", "endpoint": null,
        "price_metadata_sha256": null, "value": {"status": "unknown"}
      }]
    }
  }
}"#;

pub(super) const BUDGET_RECONCILED: &str = r#"{
  "version": 1, "sequence": 14, "job_id": "job-1", "event": {
    "type": "budget_reconciled", "reservation_sequence": 5,
    "note_sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
  }
}"#;

pub(super) const APPROVAL_REQUESTED: &str = r#"{
  "version": 1, "sequence": 7, "job_id": "job-1", "event": {
    "type": "approval_requested", "request": {
      "version": 1, "job_id": "job-1", "agent_id": null, "invocation_id": null,
      "request_sequence": 7,
      "policy_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "revocation_epoch": 0, "requested_at_unix_ms": 0, "expires_at_unix_ms": 5000,
      "scope": {"operation": "workspace_write", "managed_checkout_id": "checkout-1",
        "relative_path": "src/lib.rs"}
    }
  }
}"#;

pub(super) const APPROVAL_CLOSED_CONSUMED: &str = r#"{
  "version": 1, "sequence": 9, "job_id": "job-1", "event": {
    "type": "approval_closed", "request_sequence": 7,
    "disposition": {"disposition": "consumed", "reservation_sequence": 8}
  }
}"#;

pub(super) const CHECKOUT_CREATE_INTENT: &str = r#"{
  "version": 1, "sequence": 11, "job_id": "job-1", "agent_id": "agent-1",
  "event": {"type": "checkout_create_intent", "checkout": {
    "version": 1, "job_id": "job-1", "agent_id": "agent-1",
    "managed_checkout_id": "checkout-1", "repository_id": "repo-shared-1",
    "common_dir_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "managed_path": "/Users/saya/state/checkouts/checkout-1",
    "base_commit": "1111111111111111111111111111111111111111",
    "snapshot_id": "snapshot-1",
    "snapshot_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
    "created_by_sequence": 11, "lifecycle": "creating"
  }, "operation": {
    "version": 1, "job_id": "job-1", "operation_sequence": 11,
    "managed_checkout_id": "checkout-1", "action": "create_checkout", "state": "intent"
  }}
}"#;

pub(super) const CHECKOUT_OPERATION_INTENT: &str = r#"{
  "version": 1, "sequence": 12, "job_id": "job-1", "agent_id": "agent-1",
  "event": {"type": "checkout_operation_intent", "operation": {
    "version": 1, "job_id": "job-1", "operation_sequence": 12,
    "managed_checkout_id": "checkout-1", "action": "remove_checkout", "state": "intent"
  }}
}"#;

pub(super) const CHECKOUT_OPERATION_OUTCOME: &str = r#"{
  "version": 1, "sequence": 13, "job_id": "job-1", "agent_id": "agent-1",
  "event": {"type": "checkout_operation_outcome", "operation": {
    "version": 1, "job_id": "job-1", "operation_sequence": 12,
    "managed_checkout_id": "checkout-1", "action": "remove_checkout", "state": "uncertain"
  }}
}"#;

fn positive(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn job(value: &str) -> JobId {
    JobId::parse(value).unwrap()
}

fn event(
    sequence: u64,
    job_id: JobId,
    agent_id: Option<AgentId>,
    invocation_id: Option<saya_types::InvocationId>,
    event: CodingEventKind,
) -> CodingEventV1 {
    CodingEventV1::new(positive(sequence), job_id, agent_id, invocation_id, event).unwrap()
}

fn assert_event_matches(event: CodingEventV1, expected: &str) {
    let bytes = event.to_json_bytes().unwrap();
    assert_json_matches(&bytes, expected);
    let decoded = CodingEventV1::from_json_bytes(expected.as_bytes()).unwrap();
    assert_json_matches(&decoded.to_json_bytes().unwrap(), expected);
}

fn assert_json_matches(actual: &[u8], expected: &str) {
    let actual: Value = serde_json::from_slice(actual).unwrap();
    let expected: Value = serde_json::from_str(expected).unwrap();
    assert_eq!(actual, expected);
}

fn budget_reservation() -> BudgetReservationV1 {
    BudgetReservationV1::new(
        job("job-1"),
        positive(5),
        Some(AgentId::parse("agent-1").unwrap()),
        Some(saya_types::InvocationId::parse("inv-1").unwrap()),
        vec![BudgetCharge::new(BudgetDimension::LogicalRequests, None, None, positive(1)).unwrap()],
    )
    .unwrap()
}

fn budget_settlement() -> BudgetSettlementV1 {
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

fn approval_request() -> ApprovalRequestV1 {
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

fn checkout_record() -> ManagedCheckoutRecordV1 {
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

fn checkout_operation(
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

#[test]
fn control_events_match_fixed_nested_wire_fixtures() {
    assert_event_matches(
        event(
            5,
            job("job-1"),
            Some(AgentId::parse("agent-1").unwrap()),
            Some(saya_types::InvocationId::parse("inv-1").unwrap()),
            CodingEventKind::BudgetReserved {
                reservation: Box::new(budget_reservation()),
            },
        ),
        BUDGET_RESERVED,
    );
    assert_event_matches(
        event(
            6,
            job("job-1"),
            None,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(budget_settlement()),
            },
        ),
        BUDGET_SETTLED,
    );
    assert_event_matches(
        event(
            7,
            job("job-1"),
            None,
            None,
            CodingEventKind::ApprovalRequested {
                request: Box::new(approval_request()),
            },
        ),
        APPROVAL_REQUESTED,
    );
    assert_event_matches(
        event(
            9,
            job("job-1"),
            None,
            None,
            CodingEventKind::ApprovalClosed {
                request_sequence: positive(7),
                disposition: ApprovalDisposition::Consumed {
                    reservation_sequence: positive(8),
                },
            },
        ),
        APPROVAL_CLOSED_CONSUMED,
    );
    assert_event_matches(
        event(
            11,
            job("job-1"),
            Some(AgentId::parse("agent-1").unwrap()),
            None,
            CodingEventKind::CheckoutCreateIntent {
                checkout: Box::new(checkout_record()),
                operation: Box::new(checkout_operation(
                    11,
                    RepositoryMutationAction::CreateCheckout,
                    CheckoutOperationState::Intent,
                )),
            },
        ),
        CHECKOUT_CREATE_INTENT,
    );
    assert_event_matches(
        event(
            12,
            job("job-1"),
            Some(AgentId::parse("agent-1").unwrap()),
            None,
            CodingEventKind::CheckoutOperationIntent {
                operation: Box::new(checkout_operation(
                    12,
                    RepositoryMutationAction::RemoveCheckout,
                    CheckoutOperationState::Intent,
                )),
            },
        ),
        CHECKOUT_OPERATION_INTENT,
    );
    assert_event_matches(
        event(
            13,
            job("job-1"),
            Some(AgentId::parse("agent-1").unwrap()),
            None,
            CodingEventKind::CheckoutOperationOutcome {
                operation: Box::new(checkout_operation(
                    12,
                    RepositoryMutationAction::RemoveCheckout,
                    CheckoutOperationState::Uncertain,
                )),
            },
        ),
        CHECKOUT_OPERATION_OUTCOME,
    );
    assert_event_matches(
        event(
            14,
            job("job-1"),
            None,
            None,
            CodingEventKind::BudgetReconciled {
                reservation_sequence: positive(5),
                note_sha256: Sha256Digest::parse(NOTE_SHA).unwrap(),
            },
        ),
        BUDGET_RECONCILED,
    );
}

#[test]
fn budget_reconciled_is_a_job_event_and_refuses_an_agent() {
    let reconciled = CodingEventKind::BudgetReconciled {
        reservation_sequence: positive(5),
        note_sha256: Sha256Digest::parse(NOTE_SHA).unwrap(),
    };
    assert_event_matches(
        event(14, job("job-1"), None, None, reconciled.clone()),
        BUDGET_RECONCILED,
    );
    assert_eq!(
        CodingEventV1::new(
            positive(14),
            job("job-1"),
            Some(AgentId::parse("agent-1").unwrap()),
            None,
            reconciled,
        )
        .unwrap_err(),
        CodingEventError::InvalidEventLink
    );
}

#[test]
fn approval_closure_has_only_frozen_audit_dispositions() {
    for (disposition, expected) in [
        (ApprovalDisposition::Denied, r#"{"disposition":"denied"}"#),
        (ApprovalDisposition::Granted, r#"{"disposition":"granted"}"#),
        (ApprovalDisposition::Expired, r#"{"disposition":"expired"}"#),
        (ApprovalDisposition::Revoked, r#"{"disposition":"revoked"}"#),
        (
            ApprovalDisposition::Cancelled,
            r#"{"disposition":"cancelled"}"#,
        ),
        (
            ApprovalDisposition::Consumed {
                reservation_sequence: positive(8),
            },
            r#"{"disposition":"consumed","reservation_sequence":8}"#,
        ),
    ] {
        let event = event(
            9,
            job("job-1"),
            None,
            None,
            CodingEventKind::ApprovalClosed {
                request_sequence: positive(7),
                disposition,
            },
        );
        let wire: Value = serde_json::from_slice(&event.to_json_bytes().unwrap()).unwrap();
        assert_eq!(
            wire["event"]["disposition"],
            serde_json::from_str::<Value>(expected).unwrap()
        );
    }
}
