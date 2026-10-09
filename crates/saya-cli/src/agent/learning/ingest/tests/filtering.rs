use super::*;

#[test]
fn test_anti_self_reinforcement_drops_supplied_duplicate_inference() {
    let obj = test_object("orders");
    let supplied = vec![SuppliedContract {
        profile: "primary".to_string(),
        object: obj.qualified_name(),
        schema_state: "current",
        claims: vec![SuppliedClaim {
            claim_id: ClaimId::parse("c-1").unwrap(),
            kind: "default_time_column",
            value: "created_at".to_string(),
            column: None,
            status: ClaimStatus::Confirmed,
        }],
    }];

    // Inferred proposal with exact matching slot and value -> dropped
    let duplicate_inferred = ResolvedProposal {
        object: obj.clone(),
        profile_name: "primary".to_string(),
        slot: KnowledgeSlot::TableDefaultTime,
        value: ClaimPayload::default_time_column("created_at", None).unwrap(),
        source: ClaimOrigin::AssistantInferred,
        state: KnowledgeState::Pending,
        schema_binding: SchemaBinding::Column {
            column: "created_at".to_string(),
            requirement: ColumnRequirement::Time,
        },
    };

    let filtered = filter_anti_self_reinforcement(vec![duplicate_inferred], &supplied);
    assert!(filtered.is_empty(), "duplicate inference must be dropped");
}

#[test]
fn test_anti_self_reinforcement_permits_user_explicit_correction() {
    let obj = test_object("orders");
    let supplied = vec![SuppliedContract {
        profile: "primary".to_string(),
        object: obj.qualified_name(),
        schema_state: "current",
        claims: vec![SuppliedClaim {
            claim_id: ClaimId::parse("c-1").unwrap(),
            kind: "default_time_column",
            value: "created_at".to_string(),
            column: None,
            status: ClaimStatus::Confirmed,
        }],
    }];

    // User explicit assertion with matching or updated value -> retained
    let user_prop = ResolvedProposal {
        object: obj.clone(),
        profile_name: "primary".to_string(),
        slot: KnowledgeSlot::TableDefaultTime,
        value: ClaimPayload::default_time_column("updated_at", None).unwrap(),
        source: ClaimOrigin::UserExplicit,
        state: KnowledgeState::Active,
        schema_binding: SchemaBinding::Column {
            column: "updated_at".to_string(),
            requirement: ColumnRequirement::Time,
        },
    };

    let filtered = filter_anti_self_reinforcement(vec![user_prop.clone()], &supplied);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].state, KnowledgeState::Active);
}

#[test]
fn test_anti_self_reinforcement_suppresses_replacement_of_single_valued_slot() {
    let obj = test_object("orders");
    let supplied = vec![SuppliedContract {
        profile: "primary".to_string(),
        object: obj.qualified_name(),
        schema_state: "current",
        claims: vec![SuppliedClaim {
            claim_id: ClaimId::parse("c-1").unwrap(),
            kind: "default_time_column",
            value: "created_at".to_string(),
            column: None,
            status: ClaimStatus::Confirmed,
        }],
    }];

    // Inferred proposal targeting single-valued slot already active in supplied -> dropped
    let different_time_inferred = ResolvedProposal {
        object: obj.clone(),
        profile_name: "primary".to_string(),
        slot: KnowledgeSlot::TableDefaultTime,
        value: ClaimPayload::default_time_column("shipped_at", None).unwrap(),
        source: ClaimOrigin::AssistantInferred,
        state: KnowledgeState::Pending,
        schema_binding: SchemaBinding::Column {
            column: "shipped_at".to_string(),
            requirement: ColumnRequirement::Time,
        },
    };

    let filtered = filter_anti_self_reinforcement(vec![different_time_inferred], &supplied);
    assert!(
        filtered.is_empty(),
        "inferred replacement of active single-valued slot must be suppressed"
    );
}

#[test]
fn test_anti_self_reinforcement_allows_new_inferred_slot_or_object() {
    let obj = test_object("orders");
    let supplied = vec![SuppliedContract {
        profile: "primary".to_string(),
        object: obj.qualified_name(),
        schema_state: "current",
        claims: vec![SuppliedClaim {
            claim_id: ClaimId::parse("c-1").unwrap(),
            kind: "default_time_column",
            value: "created_at".to_string(),
            column: None,
            status: ClaimStatus::Confirmed,
        }],
    }];

    // Inferred proposal for a DIFFERENT slot (e.g. TableAlias) -> retained
    let alias_prop = ResolvedProposal {
        object: obj.clone(),
        profile_name: "primary".to_string(),
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("customer_orders").unwrap(),
        source: ClaimOrigin::AssistantInferred,
        state: KnowledgeState::Pending,
        schema_binding: SchemaBinding::Table,
    };

    // Inferred proposal for a DIFFERENT object -> retained
    let other_obj = test_object("line_items");
    let other_prop = ResolvedProposal {
        object: other_obj,
        profile_name: "primary".to_string(),
        slot: KnowledgeSlot::TableDefaultTime,
        value: ClaimPayload::default_time_column("created_at", None).unwrap(),
        source: ClaimOrigin::AssistantInferred,
        state: KnowledgeState::Pending,
        schema_binding: SchemaBinding::Column {
            column: "created_at".to_string(),
            requirement: ColumnRequirement::Time,
        },
    };

    let filtered =
        filter_anti_self_reinforcement(vec![alias_prop.clone(), other_prop.clone()], &supplied);
    assert_eq!(filtered.len(), 2);
}

#[test]
fn test_anti_self_reinforcement_dto_variant() {
    use crate::agent::learning::turn_record::SuppliedClaimDto;

    let obj = test_object("orders");
    let supplied_dto = vec![SuppliedContractDto {
        profile: "primary".to_string(),
        object: obj.qualified_name(),
        schema_state: "current".to_string(),
        claims: vec![SuppliedClaimDto {
            claim_id: "c-1".to_string(),
            kind: "default_time_column".to_string(),
            value: "created_at".to_string(),
            column: None,
            status: "confirmed".to_string(),
        }],
    }];

    let duplicate_inferred = ResolvedProposal {
        object: obj.clone(),
        profile_name: "primary".to_string(),
        slot: KnowledgeSlot::TableDefaultTime,
        value: ClaimPayload::default_time_column("created_at", None).unwrap(),
        source: ClaimOrigin::AssistantInferred,
        state: KnowledgeState::Pending,
        schema_binding: SchemaBinding::Column {
            column: "created_at".to_string(),
            requirement: ColumnRequirement::Time,
        },
    };

    let filtered = filter_anti_self_reinforcement_dto(vec![duplicate_inferred], &supplied_dto);
    assert!(filtered.is_empty(), "dto variant drops duplicate inference");
}
