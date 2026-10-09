use super::*;
use saya_store::KnowledgeItemStore;

#[tokio::test]
async fn run_extraction_empty_object_table_skips_provider() {
    let identity = test_identity("analytics");
    let registry = test_registry("analytics", &identity);
    let root = temp_root("empty_table");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let receipt = RecallReceipt::ran_empty(false);

    let record = TurnRecord {
        prompt: "hello".into(),
        assistant_answer: "hi".into(),
        object_table: TurnObjectTable::new(),
        user_corrections: Vec::new(),
        override_findings: Vec::new(),
        supplied_claims: Vec::new(),
        omitted: 0,
    };

    let provider = StaticExtractionProvider {
        response_text: r#"{"proposals": []}"#.into(),
        calls: Mutex::new(0),
    };

    let outcome = run_extraction(
        &provider,
        "test-model",
        &record,
        &registry,
        &store,
        &receipt,
    )
    .await;

    let res = outcome.dtos.expect("empty table succeeds");

    assert!(res.is_empty());
    assert_eq!(*provider.calls.lock().unwrap(), 0);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn run_extraction_success_stores_and_returns_dtos() {
    let identity = test_identity("analytics");
    let registry = test_registry("analytics", &identity);
    let root = temp_root("success");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let receipt = RecallReceipt::ran_empty(false);

    let mut object_table = TurnObjectTable::new();
    let t0 = object_table
        .register("analytics", "raw.orders", &["status".into()])
        .expect("t0 registered");

    let record = TurnRecord {
        prompt: "what is orders status".into(),
        assistant_answer: "status is pending".into(),
        object_table,
        user_corrections: Vec::new(),
        override_findings: Vec::new(),
        supplied_claims: Vec::new(),
        omitted: 0,
    };

    let json_payload = format!(
        r#"{{"proposals": [{{"object_id": "{t0}", "slot": "column:status.description", "value": "status is order state", "origin": "assistant_inferred"}}]}}"#
    );

    let provider = StaticExtractionProvider {
        response_text: json_payload,
        calls: Mutex::new(0),
    };

    let outcome = run_extraction(
        &provider,
        "test-model",
        &record,
        &registry,
        &store,
        &receipt,
    )
    .await;
    let res = outcome.dtos.expect("extraction succeeds");

    assert_eq!(res.len(), 1);
    assert_eq!(res[0].profile, "analytics");
    // Object identity is fully qualified: a bare `raw.orders` would be
    // ambiguous across catalogs.
    assert_eq!(res[0].object, "analytics.raw.orders");
    assert_eq!(res[0].column.as_deref(), Some("status"));
    assert_eq!(res[0].status, ClaimStatus::Candidate);

    // Ingestion writes to `knowledge_items` (D-3's projection), not the legacy
    // `contract_claims` table — asserting against the old store would pass only
    // if F had quietly kept writing to the thing D-3 replaced.
    let items = store
        .knowledge_for_profile(&identity)
        .await
        .expect("items listed");
    assert_eq!(items.len(), 1);

    let _ = fs::remove_dir_all(root);
}

/// The model proposes facts about the columns the turn record showed it,
/// and that list can carry SELECT aliases no table owns. Only the claim
/// naming a column the object actually has may reach the store.
#[tokio::test]
async fn run_extraction_stores_only_column_claims_the_object_actually_has() {
    let identity = test_identity("analytics");
    let registry = test_registry("analytics", &identity);
    let root = temp_root("phantom_column");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let receipt = RecallReceipt::ran_empty(false);

    let mut object_table = TurnObjectTable::new();
    let t0 = object_table
        .register(
            "analytics",
            "raw.orders",
            &["status".into(), "late_orders".into()],
        )
        .expect("t0 registered");

    let record = TurnRecord {
        prompt: "which orders are late".into(),
        assistant_answer: "late orders are those shipped after the required date".into(),
        object_table,
        user_corrections: Vec::new(),
        override_findings: Vec::new(),
        supplied_claims: Vec::new(),
        omitted: 0,
    };

    let json_payload = format!(
        r#"{{"proposals": [
                {{"object_id": "{t0}", "slot": "column:late_orders.role", "value": "measure", "origin": "assistant_inferred"}},
                {{"object_id": "{t0}", "slot": "column:status.description", "value": "the order state", "origin": "assistant_inferred"}}
            ]}}"#
    );

    let provider = StaticExtractionProvider {
        response_text: json_payload,
        calls: Mutex::new(0),
    };

    let outcome = run_extraction(
        &provider,
        "test-model",
        &record,
        &registry,
        &store,
        &receipt,
    )
    .await;
    let res = outcome.dtos.expect("extraction succeeds");

    assert_eq!(res.len(), 1, "only the claim on the real column is stored");
    assert_eq!(res[0].column.as_deref(), Some("status"));
    assert_eq!(res[0].kind, "column_description");

    let items = store
        .knowledge_for_profile(&identity)
        .await
        .expect("items listed");
    assert_eq!(
        items.len(),
        1,
        "the phantom column claim never reaches the store"
    );
    assert_eq!(
        items[0].slot,
        saya_types::KnowledgeSlot::ColumnDescription {
            column: "status".into()
        }
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn run_extraction_propagates_provider_error() {
    let identity = test_identity("analytics");
    let registry = test_registry("analytics", &identity);
    let root = temp_root("err_prop");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let receipt = RecallReceipt::ran_empty(false);

    let mut object_table = TurnObjectTable::new();
    object_table.register("analytics", "raw.orders", &["status".into()]);

    let record = TurnRecord {
        prompt: "test".into(),
        assistant_answer: "ans".into(),
        object_table,
        user_corrections: Vec::new(),
        override_findings: Vec::new(),
        supplied_claims: Vec::new(),
        omitted: 0,
    };

    let provider = ErrorProvider;
    let outcome = run_extraction(
        &provider,
        "test-model",
        &record,
        &registry,
        &store,
        &receipt,
    )
    .await;

    assert!(matches!(
        outcome.dtos,
        Err(ExtractionRunnerError::Provider(_))
    ));
    let _ = fs::remove_dir_all(root);
}
