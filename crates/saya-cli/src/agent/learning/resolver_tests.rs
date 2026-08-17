use super::*;
use crate::connection::ConnectionEntry;
use async_trait::async_trait;
use saya_connectors::DatabaseConnector;
use saya_types::{
    Column, ColumnRequirement, ConnectionError, Database, KnowledgeSlot, KnowledgeState,
    ProfileIdentity, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect, Table,
};

struct TestConnector {
    tree: SchemaTree,
}

#[async_trait]
impl DatabaseConnector for TestConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(self.tree.clone())
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult {
            columns: vec!["id".into()],
            rows: vec![serde_json::json!([1])],
            row_count: 1,
            truncated: false,
            executed_sql: req.sql,
        })
    }
}

fn sample_tree() -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![
                    Table {
                        name: "orders".into(),
                        columns: vec![
                            Column {
                                name: "id".into(),
                                data_type: "bigint".into(),
                                nullable: false,
                            },
                            Column {
                                name: "created_at".into(),
                                data_type: "timestamp".into(),
                                nullable: false,
                            },
                        ],
                    },
                    Table {
                        name: "staff".into(),
                        columns: vec![
                            Column {
                                name: "id".into(),
                                data_type: "bigint".into(),
                                nullable: false,
                            },
                            Column {
                                name: "name".into(),
                                data_type: "text".into(),
                                nullable: false,
                            },
                        ],
                    },
                ],
            }],
        }],
    }
}

fn test_registry(name: &str, identity: &ProfileIdentity, tree: SchemaTree) -> ConnectionRegistry {
    let mut reg = ConnectionRegistry::new(name);
    reg.insert(
        name,
        ConnectionEntry {
            connector: Box::new(TestConnector { tree }),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    reg
}

fn test_identity() -> ProfileIdentity {
    ProfileIdentity::parse("p-1111111111111111111111111111111111111111111111111111111111111111")
        .unwrap()
}

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

    let resolved = resolve_proposal(extracted, &table, &registry)
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

    let resolved = resolve_proposal(extracted, &table, &registry)
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

    let resolved = resolve_proposal(extracted, &table, &registry)
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

    let err = resolve_proposal(extracted, &table, &registry)
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

    let resolved = resolve_proposal(extracted, &table, &registry)
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

    let err = resolve_proposal(extracted, &table, &reg).await.unwrap_err();

    // Must NOT find "stock" in secondary profile
    assert_eq!(
        err,
        ResolutionError::UnresolvableObject("stock".to_string())
    );
}

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

    let resolved = resolve_proposal(extracted, &table, &registry)
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
    let user_resolved = resolve_proposal(user_prop, &table, &registry)
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
    let asst_resolved = resolve_proposal(asst_prop, &table, &registry)
        .await
        .unwrap();
    assert_eq!(asst_resolved.state, KnowledgeState::Pending);
    assert_eq!(asst_resolved.source, ClaimOrigin::AssistantInferred);
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

    let err = resolve_proposal(extracted, &table, &registry)
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

    let err = resolve_proposal(extracted, &table, &registry)
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

    let err = resolve_proposal(extracted, &table, &registry)
        .await
        .unwrap_err();
    assert_eq!(err, ResolutionError::InvalidSchemaBinding);
}
