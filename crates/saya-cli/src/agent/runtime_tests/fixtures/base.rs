/// A connector that never touches a live database: recall reads the store, not
/// the connector, so the registry only needs an entry to carry an identity.
struct IdleConnector;

#[async_trait]
impl saya_connectors::DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree {
            databases: vec![Database {
                name: "catalog".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![orders_table()],
                }],
            }],
        })
    }
    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-runtime-p1b-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn identity_for(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "runtime-p1b.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/runtime-p1b-test/connections.toml"),
    )
}

fn registry_for(name: &str, identity: &ProfileIdentity) -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new(name);
    registry.insert(
        name,
        ConnectionEntry {
            connector: Box::new(IdleConnector),
            dialect: SqlDialect::DuckDb,
            profile_id: Some(identity.as_str().to_string()),
        },
    );
    registry
}

async fn store_at(db: &Path, identity: &ProfileIdentity) -> SqliteStateStore {
    let store = SqliteStateStore::new(db);
    store
        .upsert_schema(identity.as_str(), &SchemaTree::default())
        .await
        .unwrap();
    store
}

fn object(identity: &ProfileIdentity, name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        identity.clone(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

fn orders_table() -> Table {
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
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

fn live_fingerprint(tree: &Table) -> saya_types::SchemaFingerprint {
    saya_types::SchemaFingerprint::of_table(DatabaseObjectKind::Table, tree)
}

fn orders_schema(identity: &ProfileIdentity) -> (ProfileIdentity, SchemaTree) {
    let table = orders_table();
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![table],
            }],
        }],
    };
    (identity.clone(), tree)
}

async fn remember_default_time_column(
    store: &SqliteStateStore,
    obj: &DatabaseObjectRef,
    _fingerprint: &saya_types::SchemaFingerprint,
    column: &str,
    status: ClaimStatus,
    origin: ClaimOrigin,
) -> ClaimId {
    use saya_types::{KnowledgeSlot, SchemaBinding};
    let payload = ClaimPayload::default_time_column(column, None).unwrap();
    let state = match status {
        ClaimStatus::Confirmed => KnowledgeState::Active,
        ClaimStatus::Candidate => KnowledgeState::Pending,
        _ => KnowledgeState::Dismissed,
    };
    let slot = KnowledgeSlot::TableDefaultTime;
    let binding = SchemaBinding::derive(&slot, &payload).expect("default_time slot/payload agree");
    let request = KnowledgeItemRequest {
        object: obj.clone(),
        slot,
        value: payload,
        source: origin,
        state,
        schema_binding_json: serde_json::to_string(&binding).unwrap(),
        fingerprint: crate::commands::unobserved_fingerprint(),
    };
    store.put_knowledge_item(request).await.unwrap();
    // Read the store-assigned `ki-` id back so the event-naming assertion
    // compares against exactly what recall supplied.
    ClaimId::parse(
        &store
            .knowledge_for_object(obj)
            .await
            .expect("knowledge items listed")
            .into_iter()
            .find(|i| i.slot == KnowledgeSlot::TableDefaultTime)
            .expect("default_time item stored")
            .id,
    )
    .expect("ki id")
}
