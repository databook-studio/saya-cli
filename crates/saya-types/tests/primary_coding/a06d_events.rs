#[path = "a06d_events/builders.rs"]
mod builders;
#[path = "a06d_events/control_events.rs"]
mod control_events;
#[path = "a06d_events/fixtures.rs"]
mod fixtures;

pub(crate) use fixtures::{
    APPROVAL_CLOSED_CONSUMED, APPROVAL_REQUESTED, BUDGET_RECONCILED, BUDGET_RESERVED,
    BUDGET_SETTLED, CHECKOUT_CREATE_INTENT, CHECKOUT_OPERATION_INTENT, CHECKOUT_OPERATION_OUTCOME,
};

use saya_types::{
    AgentId, ApprovalDisposition, CodingEventError, CodingEventKind, CodingEventV1, Sha256Digest,
};
use serde_json::Value;

use builders::{assert_event_matches, event, job, positive};
use fixtures::NOTE_SHA;

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
