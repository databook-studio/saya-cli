use saya_types::{
    BudgetCharge, BudgetError, BudgetObservation, BudgetReservationV1, BudgetSettlementV1,
    MAX_BUDGET_ENTRIES,
};
use serde_json::Value;

use super::budget_record_wire::{RESERVATION, SETTLEMENT};

fn fixed_wire(fixture: &str) -> Value {
    serde_json::from_str(fixture).unwrap()
}

fn decode_reservation(value: &Value) -> Result<BudgetReservationV1, BudgetError> {
    BudgetReservationV1::from_json_bytes(&serde_json::to_vec(value).unwrap())
}

fn decode_settlement(value: &Value) -> Result<BudgetSettlementV1, BudgetError> {
    BudgetSettlementV1::from_json_bytes(&serde_json::to_vec(value).unwrap())
}

fn repeated_entry(value: &Value, field: &str, count: usize) -> Value {
    let entry = value[field][0].clone();
    Value::Array((0..count).map(|_| entry.clone()).collect())
}

#[test]
fn reservation_deserialization_revalidates_identity_and_nested_scopes() {
    let mut no_agent = fixed_wire(RESERVATION);
    no_agent.as_object_mut().unwrap().remove("agent_id");
    assert_eq!(decode_reservation(&no_agent), Err(BudgetError::Malformed));

    let mut endpoint_mismatch = fixed_wire(RESERVATION);
    endpoint_mismatch["charges"][0]["endpoint"] = Value::String("primary".into());
    assert_eq!(
        decode_reservation(&endpoint_mismatch),
        Err(BudgetError::Malformed)
    );

    let mut price_mismatch = fixed_wire(RESERVATION);
    price_mismatch["charges"][1]["price_metadata_sha256"] =
        Value::String("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into());
    assert_eq!(
        decode_reservation(&price_mismatch),
        Err(BudgetError::Malformed)
    );

    let invalid_nested = r#"{"dimension":"input_tokens","endpoint":"../private","price_metadata_sha256":null,"reserved":1}"#;
    assert!(serde_json::from_str::<BudgetCharge>(invalid_nested).is_err());
}

#[test]
fn settlement_deserialization_revalidates_nested_scopes() {
    let mut endpoint_mismatch = fixed_wire(SETTLEMENT);
    endpoint_mismatch["observations"][0]["endpoint"] = Value::String("primary".into());
    assert_eq!(
        decode_settlement(&endpoint_mismatch),
        Err(BudgetError::Malformed)
    );

    let mut price_mismatch = fixed_wire(SETTLEMENT);
    price_mismatch["observations"][1]["price_metadata_sha256"] =
        Value::String("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into());
    assert_eq!(
        decode_settlement(&price_mismatch),
        Err(BudgetError::Malformed)
    );

    let invalid_nested = r#"{"dimension":"cost_microusd","endpoint":"primary","price_metadata_sha256":null,"value":{"status":"measured","value":1}}"#;
    assert!(serde_json::from_str::<BudgetObservation>(invalid_nested).is_err());
}

#[test]
fn both_record_wire_boundaries_reject_empty_duplicate_and_oversized_sets() {
    let mut empty_charges = fixed_wire(RESERVATION);
    empty_charges["charges"] = Value::Array(Vec::new());
    assert_eq!(
        decode_reservation(&empty_charges),
        Err(BudgetError::Malformed)
    );

    let mut duplicate_charges = fixed_wire(RESERVATION);
    duplicate_charges["charges"] = repeated_entry(&duplicate_charges, "charges", 2);
    assert_eq!(
        decode_reservation(&duplicate_charges),
        Err(BudgetError::Malformed)
    );

    let mut many_charges = fixed_wire(RESERVATION);
    many_charges["charges"] = repeated_entry(&many_charges, "charges", MAX_BUDGET_ENTRIES + 1);
    assert_eq!(
        decode_reservation(&many_charges),
        Err(BudgetError::Malformed)
    );

    let mut empty_observations = fixed_wire(SETTLEMENT);
    empty_observations["observations"] = Value::Array(Vec::new());
    assert_eq!(
        decode_settlement(&empty_observations),
        Err(BudgetError::Malformed)
    );

    let mut duplicate_observations = fixed_wire(SETTLEMENT);
    duplicate_observations["observations"] =
        repeated_entry(&duplicate_observations, "observations", 2);
    assert_eq!(
        decode_settlement(&duplicate_observations),
        Err(BudgetError::Malformed)
    );

    let mut many_observations = fixed_wire(SETTLEMENT);
    many_observations["observations"] =
        repeated_entry(&many_observations, "observations", MAX_BUDGET_ENTRIES + 1);
    assert_eq!(
        decode_settlement(&many_observations),
        Err(BudgetError::Malformed)
    );
}
