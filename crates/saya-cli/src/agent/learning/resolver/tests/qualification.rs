use super::*;

#[tokio::test]
async fn test_unqualified_staff_resolves_to_profile_schema_and_never_default() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "staff", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("employees").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let resolved = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap();

    // Must resolve to real catalog and schema from tree, NEVER "default"
    assert_eq!(resolved.object.catalog(), "analytics");
    assert_eq!(resolved.object.schema(), "public");
    assert_eq!(resolved.object.object(), "staff");
    assert_eq!(resolved.object.qualified_name(), "analytics.public.staff");
    assert!(!resolved.object.qualified_name().contains("default"));
}
#[tokio::test]
async fn test_two_part_public_staff_resolves_to_profile_catalog() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "public.staff", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("employees").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let resolved = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap();

    assert_eq!(resolved.object.catalog(), "analytics");
    assert_eq!(resolved.object.schema(), "public");
    assert_eq!(resolved.object.object(), "staff");
    assert_eq!(resolved.object.qualified_name(), "analytics.public.staff");
}
#[tokio::test]
async fn test_fully_qualified_name_is_unchanged() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table
        .register("primary", "analytics.public.staff", &[])
        .unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("employees").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let resolved = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap();

    assert_eq!(resolved.object.qualified_name(), "analytics.public.staff");
}
#[tokio::test]
async fn test_unresolvable_name_produces_error_no_fabricated_proposal() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "ghost_table", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("ghost").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let err = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap_err();

    assert_eq!(
        err,
        ResolutionError::UnresolvableObject("ghost_table".to_string())
    );
}
#[tokio::test]
async fn test_resulting_item_is_not_stale_on_arrival() {
    let tree = sample_tree();
    let identity = test_identity();
    let registry = test_registry("primary", &identity, tree.clone());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "orders", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableDefaultTime,
        value: ClaimPayload::default_time_column("created_at", None).unwrap(),
        origin: ProposalOrigin::AssistantInferred,
        confidence: 0.9,
    };

    let resolved = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap();

    // Check validity against the actual schema tree — it MUST be found
    let found = tree.find_table(
        resolved.object.catalog(),
        resolved.object.schema(),
        resolved.object.object(),
    );
    assert!(found.is_some(), "table must exist in tree");
    let t = found.unwrap();
    assert!(t.columns.iter().any(|c| c.name == "created_at"));
}
