use std::num::NonZeroU64;

use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetError, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, InvocationId, JobId, Sha256Digest,
};

const PRICE_SHA: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
pub(super) const RESERVATION: &str = r#"{"version":1,"job_id":"job-1","reservation_sequence":8,"agent_id":"agent-1","invocation_id":"inv-1","charges":[{"dimension":"logical_requests","endpoint":null,"price_metadata_sha256":null,"reserved":1},{"dimension":"input_tokens","endpoint":"primary","price_metadata_sha256":null,"reserved":4096},{"dimension":"cost_microusd","endpoint":"primary","price_metadata_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","reserved":2500000}]}"#;
pub(super) const SETTLEMENT: &str = r#"{"version":1,"job_id":"job-1","reservation_sequence":8,"settlement_sequence":12,"effect_state":"uncertain","observations":[{"dimension":"logical_requests","endpoint":null,"price_metadata_sha256":null,"value":{"status":"measured","value":0}},{"dimension":"input_tokens","endpoint":"primary","price_metadata_sha256":null,"value":{"status":"estimated","value":1200}},{"dimension":"output_tokens","endpoint":"primary","price_metadata_sha256":null,"value":{"status":"unknown"}},{"dimension":"cost_microusd","endpoint":"primary","price_metadata_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","value":{"status":"unavailable"}}]}"#;

fn assert_json_matches(actual: &[u8], expected: &str) {
    let actual: serde_json::Value = serde_json::from_slice(actual).unwrap();
    let expected: serde_json::Value = serde_json::from_str(expected).unwrap();
    assert_eq!(actual, expected);
}

fn positive(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn logical_charge() -> BudgetCharge {
    BudgetCharge::new(BudgetDimension::LogicalRequests, None, None, positive(1)).unwrap()
}

#[test]
fn reservation_and_settlement_wire_preserve_keys_and_usage_quality() {
    let reservation = BudgetReservationV1::from_json_bytes(RESERVATION.as_bytes()).unwrap();
    assert_json_matches(&reservation.to_json_bytes().unwrap(), RESERVATION);
    assert_eq!(reservation.reservation_sequence().get(), 8);
    assert_eq!(reservation.charges().len(), 3);

    let settlement = BudgetSettlementV1::from_json_bytes(SETTLEMENT.as_bytes()).unwrap();
    assert_json_matches(&settlement.to_json_bytes().unwrap(), SETTLEMENT);
    assert_eq!(settlement.reservation_sequence().get(), 8);
    assert_eq!(settlement.settlement_sequence().get(), 12);
    assert_eq!(settlement.effect_state(), BudgetEffectState::Uncertain);
    assert_eq!(settlement.observations().len(), 4);
    assert!(matches!(
        settlement.observations()[0].value(),
        BudgetObservationValue::Measured(0)
    ));
}

#[test]
fn budget_wire_versions_and_authority_fields_fail_closed() {
    let future = RESERVATION.replace("\"version\":1", "\"version\":2");
    assert_eq!(
        BudgetReservationV1::from_json_bytes(future.as_bytes()),
        Err(BudgetError::UnsupportedVersion(2))
    );
    let extra = RESERVATION.replace(
        "\"job_id\":\"job-1\",",
        "\"job_id\":\"job-1\",\"permissions\":[\"workspace_write\"],",
    );
    assert_eq!(
        BudgetReservationV1::from_json_bytes(extra.as_bytes()),
        Err(BudgetError::Malformed)
    );
    let future_settlement = SETTLEMENT.replace("\"version\":1", "\"version\":2");
    assert_eq!(
        BudgetSettlementV1::from_json_bytes(future_settlement.as_bytes()),
        Err(BudgetError::UnsupportedVersion(2))
    );
    let extra_settlement = SETTLEMENT.replace(
        "\"effect_state\":\"uncertain\",",
        "\"effect_state\":\"uncertain\",\"permissions\":[],",
    );
    assert_eq!(
        BudgetSettlementV1::from_json_bytes(extra_settlement.as_bytes()),
        Err(BudgetError::Malformed)
    );
    let zero = RESERVATION.replace("\"reserved\":1", "\"reserved\":0");
    assert_eq!(
        BudgetReservationV1::from_json_bytes(zero.as_bytes()),
        Err(BudgetError::Malformed)
    );
    let nested_extra = r#"{"dimension":"logical_requests","endpoint":null,"price_metadata_sha256":null,"reserved":1,"grant":true}"#;
    assert!(serde_json::from_str::<BudgetCharge>(nested_extra).is_err());
}

#[test]
fn budget_scopes_identities_and_bounds_are_validated() {
    assert_eq!(
        BudgetCharge::new(
            BudgetDimension::LogicalRequests,
            Some("primary".to_string()),
            None,
            positive(1),
        ),
        Err(BudgetError::EndpointScopeMismatch)
    );
    assert_eq!(
        BudgetCharge::new(
            BudgetDimension::InputTokens,
            Some("../endpoint".to_string()),
            None,
            positive(1),
        ),
        Err(BudgetError::InvalidEndpoint)
    );
    assert_eq!(
        BudgetCharge::new(
            BudgetDimension::CostMicrousd,
            Some("primary".to_string()),
            None,
            positive(1),
        ),
        Err(BudgetError::InvalidPriceMetadata)
    );
    assert_eq!(
        BudgetCharge::new(
            BudgetDimension::InputTokens,
            Some("primary".to_string()),
            Some(Sha256Digest::parse(PRICE_SHA).unwrap()),
            positive(1),
        ),
        Err(BudgetError::InvalidPriceMetadata)
    );

    let charge = logical_charge();
    assert_eq!(
        BudgetReservationV1::new(
            JobId::parse("job-1").unwrap(),
            positive(8),
            None,
            Some(InvocationId::parse("inv-1").unwrap()),
            vec![charge.clone()],
        ),
        Err(BudgetError::InvalidIdentity)
    );
}
