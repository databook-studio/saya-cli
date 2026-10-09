use super::*;

#[tokio::test]
async fn test_resolve_proposal_derives_schema_binding() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table
        .register("primary", "analytics.public.orders", &[])
        .unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableDefaultTime,
        value: ClaimPayload::default_time_column("created_at", None).unwrap(),
        origin: ProposalOrigin::AssistantInferred,
        confidence: 0.95,
    };

    let resolved = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap();
    assert_eq!(resolved.profile_name, "primary");
    assert_eq!(resolved.object.qualified_name(), "analytics.public.orders");
    assert_eq!(resolved.slot, KnowledgeSlot::TableDefaultTime);
    assert_eq!(resolved.state, KnowledgeState::Pending);
    assert_eq!(resolved.source, ClaimOrigin::AssistantInferred);
    assert_eq!(
        resolved.schema_binding,
        SchemaBinding::Column {
            column: "created_at".to_string(),
            requirement: ColumnRequirement::Time,
        }
    );
}
#[tokio::test]
async fn test_resolve_assigns_active_for_user_and_pending_for_assistant() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "public.orders", &[]).unwrap();

    // User explicit -> Active
    let user_prop = ExtractedProposal {
        object_id: id.clone(),
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("user_accounts").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };
    let user_resolved = resolve_proposal(
        user_prop,
        "remember that orders tracks customer accounts",
        &table,
        &registry,
    )
    .await
    .unwrap();
    assert_eq!(user_resolved.state, KnowledgeState::Active);
    assert_eq!(user_resolved.source, ClaimOrigin::UserExplicit);
    assert_eq!(user_resolved.schema_binding, SchemaBinding::Table);

    // Assistant inferred -> Pending
    let asst_prop = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("user_accounts").unwrap(),
        origin: ProposalOrigin::AssistantInferred,
        confidence: 0.8,
    };
    let asst_resolved = resolve_proposal(asst_prop, "show me the orders", &table, &registry)
        .await
        .unwrap();
    assert_eq!(asst_resolved.state, KnowledgeState::Pending);
    assert_eq!(asst_resolved.source, ClaimOrigin::AssistantInferred);
}
