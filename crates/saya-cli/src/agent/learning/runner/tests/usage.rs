use super::request::UsageExtractionProvider;
use super::*;

/// A successful extraction surfaces the usage the provider reported, so the
/// recorder can fold it into the learning total rather than dropping it.
#[tokio::test]
async fn run_extraction_surfaces_usage_on_success() {
    let identity = test_identity("analytics");
    let registry = test_registry("analytics", &identity);
    let root = temp_root("usage_ok");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let receipt = RecallReceipt::ran_empty(false);

    let mut object_table = TurnObjectTable::new();
    object_table.register("analytics", "raw.orders", &["status".into()]);

    let record = TurnRecord {
        prompt: "what is orders status".into(),
        assistant_answer: "status is pending".into(),
        object_table,
        user_corrections: Vec::new(),
        override_findings: Vec::new(),
        supplied_claims: Vec::new(),
        omitted: 0,
    };

    let provider = UsageExtractionProvider {
        response_text: r#"{"proposals": []}"#.into(),
        usage: TokenUsage::new(40, 10),
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

    outcome.dtos.expect("extraction succeeds");
    assert_eq!(
        outcome.usage,
        Some(TokenUsage::new(40, 10)),
        "a successful extraction must surface the provider-reported usage"
    );
    let _ = fs::remove_dir_all(root);
}

/// A failed extraction that still received a response surfaces the usage:
/// tokens may have been billed before the parse failed, and the accounting
/// must not be tied to the success path.
#[tokio::test]
async fn run_extraction_surfaces_usage_on_parse_failure() {
    let identity = test_identity("analytics");
    let registry = test_registry("analytics", &identity);
    let root = temp_root("usage_parse_err");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let receipt = RecallReceipt::ran_empty(false);

    let mut object_table = TurnObjectTable::new();
    object_table.register("analytics", "raw.orders", &["status".into()]);

    let record = TurnRecord {
        prompt: "what is orders status".into(),
        assistant_answer: "status is pending".into(),
        object_table,
        user_corrections: Vec::new(),
        override_findings: Vec::new(),
        supplied_claims: Vec::new(),
        omitted: 0,
    };

    let provider = UsageExtractionProvider {
        response_text: "definitely not json".into(),
        usage: TokenUsage::new(40, 10),
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

    assert!(
        matches!(outcome.dtos, Err(ExtractionRunnerError::Extraction(_))),
        "malformed content must fail extraction, got: {:?}",
        outcome.dtos
    );
    assert_eq!(
        outcome.usage,
        Some(TokenUsage::new(40, 10)),
        "a failed extraction that received a response must still surface its usage"
    );
    let _ = fs::remove_dir_all(root);
}
