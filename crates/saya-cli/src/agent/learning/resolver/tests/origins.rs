use super::*;

/// The origin field arrives from the model's own output, so a claim of a user
/// assertion must be corroborated by the turn itself before it can confirm a
/// fact: an unasserted turn resolves the proposal like any inference.
#[tokio::test]
async fn test_user_explicit_without_assertion_resolves_pending_not_active() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "public.orders", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("customer_orders").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let resolved = resolve_proposal(
        extracted,
        "show me the orders shipped last month",
        &table,
        &registry,
    )
    .await
    .unwrap();

    assert_eq!(resolved.state, KnowledgeState::Pending);
    assert_eq!(resolved.source, ClaimOrigin::AssistantInferred);
}
#[tokio::test]
async fn test_user_explicit_with_assertion_resolves_active() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "public.orders", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("customer_orders").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let resolved = resolve_proposal(
        extracted,
        "remember that we call this table customer_orders",
        &table,
        &registry,
    )
    .await
    .unwrap();

    assert_eq!(resolved.state, KnowledgeState::Active);
    assert_eq!(resolved.source, ClaimOrigin::UserExplicit);
}
#[tokio::test]
async fn test_assistant_inferred_is_pending_with_or_without_assertion() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "public.orders", &[]).unwrap();

    let make_proposal = || ExtractedProposal {
        object_id: id.clone(),
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("customer_orders").unwrap(),
        origin: ProposalOrigin::AssistantInferred,
        confidence: 0.8,
    };

    let without = resolve_proposal(make_proposal(), "list the orders", &table, &registry)
        .await
        .unwrap();
    assert_eq!(without.state, KnowledgeState::Pending);
    assert_eq!(without.source, ClaimOrigin::AssistantInferred);

    let with = resolve_proposal(
        make_proposal(),
        "remember that we call this table customer_orders",
        &table,
        &registry,
    )
    .await
    .unwrap();
    assert_eq!(with.state, KnowledgeState::Pending);
    assert_eq!(with.source, ClaimOrigin::AssistantInferred);
}
