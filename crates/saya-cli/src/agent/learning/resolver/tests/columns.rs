use super::*;

/// A join observation hands every referenced object the result set's whole
/// column list, so a SELECT alias can end up on a table that never had it.
/// A proposal naming such a column must be refused: the schema is the only
/// authority on what the object actually owns.
#[tokio::test]
async fn test_proposal_naming_absent_column_is_refused() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table
        .register("primary", "orders", &["late_orders".into()])
        .unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::ColumnRole {
            column: "late_orders".into(),
        },
        value: ClaimPayload::column_role("late_orders", ColumnRole::Measure, None).unwrap(),
        origin: ProposalOrigin::AssistantInferred,
        confidence: 0.8,
    };

    let err = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        ResolutionError::UnknownColumn(
            "analytics.public.orders".to_string(),
            "late_orders".to_string()
        )
    );
}
#[tokio::test]
async fn test_proposal_naming_real_column_still_resolves() {
    let identity = test_identity();
    let registry = test_registry("primary", &identity, sample_tree());
    let mut table = TurnObjectTable::new();
    let id = table
        .register("primary", "orders", &["created_at".into()])
        .unwrap();

    let extracted = ExtractedProposal {
        object_id: id,
        slot: KnowledgeSlot::ColumnDescription {
            column: "created_at".into(),
        },
        value: ClaimPayload::column_description("created_at", "when the order was placed").unwrap(),
        origin: ProposalOrigin::AssistantInferred,
        confidence: 0.8,
    };

    let resolved = resolve_proposal(extracted, "show me the orders", &table, &registry)
        .await
        .unwrap();
    assert_eq!(resolved.object.object(), "orders");
    assert_eq!(
        resolved.schema_binding,
        SchemaBinding::Column {
            column: "created_at".to_string(),
            requirement: ColumnRequirement::Exists,
        }
    );
}
/// A join's result columns land on every referenced object's entry, so the
/// extraction prompt legitimately shows one table columns that belong to the
/// other. A claim about a column one of them really owns must still resolve —
/// refusing cross-attributed names wholesale would drop real facts.
#[tokio::test]
async fn test_column_claim_on_real_column_survives_join_attribution() {
    let join_tree = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![
                    Table {
                        name: "orders".into(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                        columns: vec![
                            Column {
                                name: "orderid".into(),
                                data_type: "int".into(),
                                nullable: false,
                            },
                            Column {
                                name: "customerid".into(),
                                data_type: "int".into(),
                                nullable: false,
                            },
                            Column {
                                name: "requireddate".into(),
                                data_type: "timestamp".into(),
                                nullable: false,
                            },
                        ],
                    },
                    Table {
                        name: "customers".into(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                        columns: vec![
                            Column {
                                name: "customerid".into(),
                                data_type: "int".into(),
                                nullable: false,
                            },
                            Column {
                                name: "companyname".into(),
                                data_type: "text".into(),
                                nullable: false,
                            },
                        ],
                    },
                ],
            }],
        }],
    };
    let identity = test_identity();
    let registry = test_registry("primary", &identity, join_tree);
    let mut table = TurnObjectTable::new();
    let orders = table
        .register(
            "primary",
            "orders",
            &["orderid".into(), "customerid".into(), "companyname".into()],
        )
        .unwrap();
    let customers = table
        .register(
            "primary",
            "customers",
            &["orderid".into(), "customerid".into(), "companyname".into()],
        )
        .unwrap();

    let orders_claim = ExtractedProposal {
        object_id: orders,
        slot: KnowledgeSlot::ColumnRole {
            column: "requireddate".into(),
        },
        value: ClaimPayload::column_role("requireddate", ColumnRole::Timestamp, None).unwrap(),
        origin: ProposalOrigin::AssistantInferred,
        confidence: 0.8,
    };
    let customers_claim = ExtractedProposal {
        object_id: customers,
        slot: KnowledgeSlot::ColumnDescription {
            column: "companyname".into(),
        },
        value: ClaimPayload::column_description("companyname", "the customer's trading name")
            .unwrap(),
        origin: ProposalOrigin::AssistantInferred,
        confidence: 0.8,
    };

    let resolved_orders = resolve_proposal(orders_claim, "show me the orders", &table, &registry)
        .await
        .unwrap();
    assert_eq!(resolved_orders.object.object(), "orders");
    let resolved_customers =
        resolve_proposal(customers_claim, "show me the orders", &table, &registry)
            .await
            .unwrap();
    assert_eq!(resolved_customers.object.object(), "customers");
}
