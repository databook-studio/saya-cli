/// `mode = Assisted` supplies confirmed claims for override detection.
fn a1_memory() -> ResolvedMemory {
    ResolvedMemory {
        mode: MemoryMode::Assisted,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// `mode = Assisted` supplies both confirmed and candidate claims.
fn a1_include_candidates_memory() -> ResolvedMemory {
    ResolvedMemory {
        mode: MemoryMode::Assisted,
        max_contracts: 5,
        max_claims_per_contract: 12,
        max_context_bytes: 16384,
    }
}

/// A provider that issues one `bounded_sql_query` call with `sql` on the first
/// request and a text answer on every subsequent one — a one-query turn that
/// completes, so the runtime drains the override log and emits after the loop.
struct QueryProvider {
    sql: &'static str,
    calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for QueryProvider {
    fn name(&self) -> &str {
        "query-once"
    }
    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        let mut calls = self.calls.lock().unwrap();
        if *calls == 0 {
            *calls = 1;
            Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({ "sql": self.sql }),
                }],
                tool_call_id: None,
            }))
        } else {
            Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
        }
    }
}

/// The live override statement: a different time-named column referenced on the
/// claimed object, the claimed column absent.
const OVERRIDE_SQL: &str = "SELECT rental_date FROM orders WHERE rental_date > '2024-01-01'";

/// Seeds a confirmed `default_time_column` claim of `return_date` on
/// `catalog.public.orders` under a matching cached schema, returning the temp
/// root, the claim id, and the open store. Mirrors the test-1 harness.
///
/// D-4 NOTE: the claim's binding is `Column { return_date, Time }`, so the
/// cached schema must carry `return_date` as a temporal column for the item to
/// read `current` (and so reach the model, where override detection sees it).
/// The old whole-table fingerprint model classified the claim `Current` from
/// the fingerprint match alone; D-4 classifies from the binding, so the schema
/// must name the bound column.
async fn a1_turn_setup(
    status: ClaimStatus,
    origin: ClaimOrigin,
) -> (PathBuf, ClaimId, SqliteStateStore) {
    let root = temp_root("a1");
    let db = root.join("state.sqlite3");
    let identity = identity_for("analytics");
    let store = store_at(&db, &identity).await;
    let obj = object(&identity, "orders");
    let tree = SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "orders".into(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: vec![
                        Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        },
                        Column {
                            name: "return_date".into(),
                            data_type: "timestamp".into(),
                            nullable: false,
                        },
                    ],
                }],
            }],
        }],
    };
    store.upsert_schema(identity.as_str(), &tree).await.unwrap();
    let fp = live_fingerprint(&orders_table());
    let claim_id =
        remember_default_time_column(&store, &obj, &fp, "return_date", status, origin).await;
    (root, claim_id, store)
}

/// The single `KnowledgeOverridden` event in `captured`, if any. Detection
/// emits at most one event per turn carrying every finding.
fn one_overridden(captured: &[AgentEvent]) -> Option<&[OverrideFindingDto]> {
    captured.iter().find_map(|event| match event {
        AgentEvent::KnowledgeOverridden { findings } => Some(findings.as_slice()),
        _ => None,
    })
}

// Test 1: a turn whose SQL contradicts a supplied confirmed claim emits one
// `KnowledgeOverridden` naming it.
