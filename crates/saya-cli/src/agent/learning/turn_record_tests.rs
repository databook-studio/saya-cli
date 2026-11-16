//! Tests for the bounded turn record assembly.

use super::*;
use crate::agent::tools::{ObservationOutcome, ToolObservation};
use saya_types::ClaimId;

#[test]
fn test_turn_record_bounds_input_strings() {
    let huge_prompt = "p".repeat(50_000);
    let huge_answer = "a".repeat(50_000);
    let obs = DrainedObservations {
        observations: Vec::new(),
        truncated: false,
    };
    let registry = ConnectionRegistry::new("primary");

    let record = TurnRecord::assemble(&huge_prompt, &huge_answer, &registry, &obs, None, &[]);
    assert_eq!(record.prompt.len(), MAX_PROMPT_BYTES);
    assert_eq!(record.assistant_answer.len(), MAX_ANSWER_BYTES);
}

#[test]
fn test_extract_user_corrections() {
    let prompt = "Actually, orders are sorted by date. No, use return_date.";
    let corrections = extract_user_corrections(prompt);
    assert_eq!(corrections.len(), 2);
}

#[test]
fn test_dto_conversion_from_supplied_contract() {
    let contract = SuppliedContract {
        profile: "p1".into(),
        object: "catalog.public.orders".into(),
        schema_state: "current",
        claims: vec![crate::contracts::SuppliedClaim {
            claim_id: ClaimId::parse("c-1").unwrap(),
            kind: "default_time_column",
            value: "created_at".into(),
            column: Some("created_at".into()),
            status: saya_types::ClaimStatus::Confirmed,
        }],
    };

    let dto = SuppliedContractDto::from(&contract);
    assert_eq!(dto.profile, "p1");
    assert_eq!(dto.claims.len(), 1);
    assert_eq!(dto.claims[0].claim_id, "c-1");
    assert_eq!(dto.claims[0].value, "created_at");
}

#[test]
fn assembled_record_stays_within_serialized_budget_and_reports_omissions() {
    let obs = DrainedObservations {
        observations: (0..32)
            .map(|i| ToolObservation {
                tool: format!("tool-{i}"),
                outcome: ObservationOutcome::Succeeded,
                profile: None,
                objects: vec![vec![format!("catalog.public.{}", "orders".repeat(500))]],
                columns: (0..32)
                    .map(|n| format!("column_{}", "x".repeat(200 + n)))
                    .collect(),
                row_count: None,
                truncated: None,
                references_partial: false,
            })
            .collect(),
        truncated: true,
    };
    let record = TurnRecord::assemble(
        &"prompt ".repeat(2_000),
        &"answer ".repeat(2_000),
        &ConnectionRegistry::new("primary"),
        &obs,
        None,
        &[],
    );

    let bytes = serde_json::to_vec(&record).expect("turn records are JSON serializable");
    assert!(
        bytes.len() <= MAX_TURN_RECORD_BYTES,
        "{} > cap",
        bytes.len()
    );
    assert!(
        record.omitted() > 0,
        "the dropped evidence must be reported"
    );
}
