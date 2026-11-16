use super::*;

#[test]
fn recall_off_emits_the_off_outcome() {
    // recall = off: SAYA did not look. Distinct from the privacy gate.
    let receipt = RecallReceipt::configured_off();
    assert_eq!(receipt.kind, RecallOutcomeKind::ConfiguredOff);
    let event = knowledge_supplied_event(&receipt);
    assert!(matches!(
        event,
        AgentEvent::KnowledgeSupplied {
            outcome: KnowledgeOutcome::Off,
            contracts,
            dropped_by_bounds: 0,
        } if contracts.is_empty()
    ));
}

#[test]
fn privacy_gate_closed_emits_the_skipped_outcome() {
    // Privacy gate closed: SAYA was not allowed to look. No store query.
    let receipt = RecallReceipt::privacy_gate_closed();
    assert_eq!(receipt.kind, RecallOutcomeKind::PrivacyGateClosed);
    let event = knowledge_supplied_event(&receipt);
    assert!(matches!(
        event,
        AgentEvent::KnowledgeSupplied {
            outcome: KnowledgeOutcome::Skipped,
            contracts,
            dropped_by_bounds: 0,
        } if contracts.is_empty()
    ));
}

#[test]
fn recall_ran_and_found_nothing_emits_ran_distinct_from_off_and_skipped() {
    // Recall ran and matched nothing: a Ran receipt with empty supplied. This
    // is the third state — distinct from Off (did not look) and Skipped (not
    // allowed to look). The three outcomes must not collapse.
    let receipt = RecallReceipt::ran_empty(false);
    let event = knowledge_supplied_event(&receipt);
    assert!(matches!(
        event,
        AgentEvent::KnowledgeSupplied {
            outcome: KnowledgeOutcome::Ran { store_unavailable: false },
            contracts,
            dropped_by_bounds: 0,
        } if contracts.is_empty()
    ));
    // The three are distinguishable.
    let off = knowledge_supplied_event(&RecallReceipt::configured_off());
    let skipped = knowledge_supplied_event(&RecallReceipt::privacy_gate_closed());
    let ran = knowledge_supplied_event(&receipt);
    assert_ne!(outcome_of(&off), outcome_of(&skipped));
    assert_ne!(outcome_of(&skipped), outcome_of(&ran));
    assert_ne!(outcome_of(&off), outcome_of(&ran));
}

#[test]
fn a_candidate_claims_status_survives_into_the_event() {
    let claim_id = ClaimId::parse("c-candidate-001").unwrap();
    let receipt = RecallReceipt {
        kind: RecallOutcomeKind::Ran {
            store_unavailable: false,
        },
        supplied: vec![crate::contracts::SuppliedContract {
            profile: "analytics".into(),
            object: "catalog.public.orders".into(),
            schema_state: "current",
            claims: vec![crate::contracts::SuppliedClaim {
                claim_id: claim_id.clone(),
                kind: "default_time_column",
                value: "created_at".into(),
                column: Some("created_at".into()),
                status: ClaimStatus::Candidate,
            }],
        }],
        dropped_by_bounds: 0,
    };
    let event = knowledge_supplied_event(&receipt);
    let contracts = match event {
        AgentEvent::KnowledgeSupplied { contracts, .. } => contracts,
        _ => panic!("expected KnowledgeSupplied"),
    };
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].claims.len(), 1);
    assert_eq!(contracts[0].claims[0].claim_id, claim_id);
    // The candidate status survives as Candidate, not flattened to confirmed.
    assert_eq!(contracts[0].claims[0].status, ClaimStatus::Candidate);
    // Distinct from a confirmed claim's status.
    assert_ne!(contracts[0].claims[0].status, ClaimStatus::Confirmed);
}
