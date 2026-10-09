/// The conversation byte budget these recall tests build against — the same role
/// the loop's `context_byte_budget` plays in production.
const RECALL_BYTE_BUDGET: usize = 32 * 1024;

/// Drives [`recall_context_blocks`] with a fixed byte budget so the tests do not
/// each repeat the budget argument. The budget is the loop's
/// `context_byte_budget`, which bounds the rendered block alongside the prompt.
async fn recall_blocks(
    prompt: &str,
    system_prompt: Option<&str>,
    allow_database_context: bool,
    recall_mode: RecallMode,
    bounds: RecallBounds,
    registry: &ConnectionRegistry,
    state_db: Option<&SqliteStateStore>,
) -> (Vec<ContextBlock>, RecallReceipt) {
    recall_context_blocks(
        prompt,
        system_prompt,
        allow_database_context,
        recall_mode,
        bounds,
        registry,
        state_db,
        RECALL_BYTE_BUDGET,
    )
    .await
}

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

/// A connector that never touches a live database: recall reads the store, not
/// the connector, so the registry only needs an entry to carry an identity.
struct IdleConnector;

#[async_trait]
impl DatabaseConnector for IdleConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
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
        "saya-recall-context-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn identity_for(name: &str) -> ProfileIdentity {
    crate::profile_identity::profile_identity(
        name,
        &DatabaseProfile::DuckDb {
            path: "recall-context.duckdb".into(),
            read_only: Some(true),
        },
        Path::new("/recall-context-test/connections.toml"),
    )
}

/// A registry whose single primary connection carries a real identity.
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
