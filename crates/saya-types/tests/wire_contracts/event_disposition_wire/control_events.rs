use saya_types::{
    AgentId, ApprovalDisposition, CheckoutOperationState, CodingEventKind,
    RepositoryMutationAction, Sha256Digest,
};

use super::builders::{
    approval_request, assert_event_matches, budget_reservation, budget_settlement,
    checkout_operation, checkout_record, event, job, positive,
};
use super::fixtures::{
    APPROVAL_CLOSED_CONSUMED, APPROVAL_REQUESTED, BUDGET_RECONCILED, BUDGET_RESERVED,
    BUDGET_SETTLED, CHECKOUT_CREATE_INTENT, CHECKOUT_OPERATION_INTENT, CHECKOUT_OPERATION_OUTCOME,
    NOTE_SHA,
};

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
