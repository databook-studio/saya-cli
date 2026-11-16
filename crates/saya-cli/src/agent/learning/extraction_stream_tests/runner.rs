use super::*;

/// The runner-level stop path: a prose reply surfaces through
/// `ExtractionOutcome::failed` as `NotJson`, so the user sees the existing
/// "memory not recorded · extraction failed" line and the trace prints the
/// cause.
#[tokio::test]
async fn a_prose_extraction_reply_makes_run_extraction_return_a_failed_outcome() {
    let registry = ConnectionRegistry::new("analytics");
    let root = temp_root("prose_runner");
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

    let provider = scripted(vec![delta(PROSE), done()], false);

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
        matches!(
            outcome.dtos,
            Err(ExtractionRunnerError::Extraction(ExtractionError::NotJson))
        ),
        "a prose reply must fail the run as NotJson, got: {:?}",
        outcome.dtos
    );

    let _ = fs::remove_dir_all(root);
}
