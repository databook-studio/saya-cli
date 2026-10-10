use saya_types::{CodingEventError, CodingEventV1};
use serde_json::{Value, json};

use super::a06d_events::{
    APPROVAL_CLOSED_CONSUMED, APPROVAL_REQUESTED, BUDGET_RESERVED, BUDGET_SETTLED,
    CHECKOUT_CREATE_INTENT, CHECKOUT_OPERATION_INTENT, CHECKOUT_OPERATION_OUTCOME,
};

fn event_wire(fixture: &str) -> Value {
    serde_json::from_str(fixture).unwrap()
}

fn decode(value: Value) -> Result<CodingEventV1, CodingEventError> {
    CodingEventV1::from_json_bytes(&serde_json::to_vec(&value).unwrap())
}

#[test]
fn budget_request_and_checkout_outer_links_must_match_nested_records() {
    let mut wrong_reservation_sequence = event_wire(BUDGET_RESERVED);
    wrong_reservation_sequence["sequence"] = json!(4);
    assert_eq!(
        decode(wrong_reservation_sequence),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut wrong_reservation_actor = event_wire(BUDGET_RESERVED);
    wrong_reservation_actor["agent_id"] = json!("agent-other");
    assert_eq!(
        decode(wrong_reservation_actor),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut actor_on_settlement = event_wire(BUDGET_SETTLED);
    actor_on_settlement["agent_id"] = json!("agent-1");
    assert_eq!(
        decode(actor_on_settlement),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut wrong_request_sequence = event_wire(APPROVAL_REQUESTED);
    wrong_request_sequence["sequence"] = json!(6);
    assert_eq!(
        decode(wrong_request_sequence),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut wrong_checkout_owner = event_wire(CHECKOUT_CREATE_INTENT);
    wrong_checkout_owner["agent_id"] = json!("agent-other");
    assert_eq!(
        decode(wrong_checkout_owner),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut wrong_checkout_id = event_wire(CHECKOUT_CREATE_INTENT);
    wrong_checkout_id["event"]["operation"]["managed_checkout_id"] = json!("checkout-other");
    assert_eq!(
        decode(wrong_checkout_id),
        Err(CodingEventError::InvalidEventLink)
    );
}

#[test]
fn approval_consumption_and_checkout_outcomes_use_distinct_event_sequences() {
    let mut reservation_after_closure = event_wire(APPROVAL_CLOSED_CONSUMED);
    reservation_after_closure["event"]["disposition"]["reservation_sequence"] = json!(9);
    assert_eq!(
        decode(reservation_after_closure),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut mixed_approval_actor = event_wire(APPROVAL_CLOSED_CONSUMED);
    mixed_approval_actor["agent_id"] = json!("agent-1");
    assert_eq!(
        decode(mixed_approval_actor),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut create_intent_as_operation_intent = event_wire(CHECKOUT_CREATE_INTENT);
    create_intent_as_operation_intent["event"]["operation"]["state"] = json!("completed");
    assert_eq!(
        decode(create_intent_as_operation_intent),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut operation_intent_with_wrong_outer_sequence = event_wire(CHECKOUT_OPERATION_INTENT);
    operation_intent_with_wrong_outer_sequence["sequence"] = json!(13);
    assert_eq!(
        decode(operation_intent_with_wrong_outer_sequence),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut outcome_reuses_intent_sequence = event_wire(CHECKOUT_OPERATION_OUTCOME);
    outcome_reuses_intent_sequence["sequence"] = json!(12);
    assert_eq!(
        decode(outcome_reuses_intent_sequence),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut outcome_still_intent = event_wire(CHECKOUT_OPERATION_OUTCOME);
    outcome_still_intent["event"]["operation"]["state"] = json!("intent");
    assert_eq!(
        decode(outcome_still_intent),
        Err(CodingEventError::InvalidEventLink)
    );
}

#[test]
fn control_events_reject_identity_and_outcome_shape_confusion() {
    let mut wrong_job: Value = event_wire(BUDGET_RESERVED);
    wrong_job["event"]["reservation"]["job_id"] = json!("job-other");
    assert_eq!(decode(wrong_job), Err(CodingEventError::InvalidEventLink));

    let mut wrong_invocation = event_wire(APPROVAL_REQUESTED);
    wrong_invocation["agent_id"] = json!("agent-1");
    wrong_invocation["invocation_id"] = json!("inv-1");
    assert_eq!(
        decode(wrong_invocation),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut outcome_with_invocation = event_wire(CHECKOUT_OPERATION_OUTCOME);
    outcome_with_invocation["invocation_id"] = json!("inv-1");
    assert_eq!(
        decode(outcome_with_invocation),
        Err(CodingEventError::InvalidEventLink)
    );

    let mut invalid_create_sequence = event_wire(CHECKOUT_CREATE_INTENT);
    invalid_create_sequence["event"]["checkout"]["created_by_sequence"] = json!(10);
    assert_eq!(
        decode(invalid_create_sequence),
        Err(CodingEventError::InvalidEventLink)
    );
}

#[test]
fn control_event_wire_rejects_unknown_fields_versions_and_reusable_grants() {
    let mut future_version = event_wire(BUDGET_SETTLED);
    future_version["version"] = json!(2);
    assert_eq!(
        decode(future_version),
        Err(CodingEventError::UnsupportedVersion(2))
    );

    let mut outer_extra = event_wire(BUDGET_SETTLED);
    outer_extra["event"]["extra"] = json!(true);
    assert_eq!(decode(outer_extra), Err(CodingEventError::Malformed));

    let mut nested_extra = event_wire(APPROVAL_REQUESTED);
    nested_extra["event"]["request"]["extra"] = json!(true);
    assert_eq!(decode(nested_extra), Err(CodingEventError::Malformed));

    let mut grant = event_wire(APPROVAL_CLOSED_CONSUMED);
    grant["event"]["disposition"] = json!({ "disposition": "allowed_once" });
    assert_eq!(decode(grant), Err(CodingEventError::Malformed));
}
