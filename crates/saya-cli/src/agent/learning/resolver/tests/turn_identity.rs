use super::*;

#[tokio::test]
async fn test_must_not_widen_resolution_to_guess_across_profiles() {
    let identity_pri = test_identity();
    let identity_sec = ProfileIdentity::parse(
        "p-2222222222222222222222222222222222222222222222222222222222222222",
    )
    .unwrap();

    let mut reg = ConnectionRegistry::new("primary");
    reg.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(TestConnector {
                tree: sample_tree(),
            }),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity_pri.as_str().to_string()),
        },
    );

    let sec_tree = SchemaTree {
        databases: vec![Database {
            name: "warehouse".into(),
            schemas: vec![Schema {
                name: "inventory".into(),
                tables: vec![Table {
                    name: "stock".into(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: vec![],
                }],
            }],
        }],
    };
    reg.insert(
        "secondary",
        ConnectionEntry {
            connector: Box::new(TestConnector { tree: sec_tree }),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity_sec.as_str().to_string()),
        },
    );

    let mut table = TurnObjectTable::new();
    // Registering "stock" under "primary" profile (which does NOT have "stock")
    let id = table.register("primary", "stock", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("stock_items").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let err = resolve_proposal(extracted, "show me the orders", &table, &reg)
        .await
        .unwrap_err();

    // Must NOT find "stock" in secondary profile
    assert_eq!(
        err,
        ResolutionError::UnresolvableObject("stock".to_string())
    );
}
#[tokio::test]
async fn test_resolve_rejects_unknown_turn_object_id() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let table = TurnObjectTable::new();

    let extracted = ExtractedProposal {
        object_id: TurnObjectId::new(99),
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("orders").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let err = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap_err();
    assert!(matches!(err, ResolutionError::ObjectIdNotFound(_)));
}
#[tokio::test]
async fn test_resolve_rejects_missing_profile_connection() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("secondary", "orders", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableAlias,
        value: ClaimPayload::table_alias("orders").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let err = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap_err();
    assert!(matches!(err, ResolutionError::ConnectionError(_, _)));
}
#[tokio::test]
async fn test_resolve_rejects_mismatched_slot_and_payload() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table.register("primary", "orders", &[]).unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::TableDescription,
        // Value is an alias payload, not table description!
        value: ClaimPayload::table_alias("orders").unwrap(),
        origin: ProposalOrigin::UserExplicit,
        confidence: 1.0,
    };

    let err = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap_err();
    assert_eq!(err, ResolutionError::InvalidSchemaBinding);
}
