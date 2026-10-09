#[tokio::test]
async fn run_outcome_carries_result_and_evidence_on_success() {
    let root = temp_root("outcome-success");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));

    let (outcome, out, err) = run_outcome(&repo, run_command(&id), &runtime, &state_db).await;
    assert_eq!(outcome.code, 0, "out: {out} err: {err}");
    let replay = outcome
        .replay
        .expect("a successful replay carries its result and evidence");
    assert_eq!(replay.sql, "SELECT id, label FROM events ORDER BY id");
    assert_eq!(replay.connection, "local");
    assert_eq!(replay.result.columns, ["id", "label"]);
    assert_eq!(replay.result.row_count, 2);
    assert_eq!(replay.evidence.returned_rows, 2);
    assert_eq!(replay.evidence.connection_label, "local");
    assert_eq!(replay.evidence.dialect, SqlDialect::Sqlite);
    assert!(matches!(replay.evidence.scope, ResultScope::Full));
    assert_eq!(
        replay.evidence.source,
        EvidenceSource::SavedInvestigation {
            id: id.clone(),
            revision: 1,
        }
    );
    // The same events are still emitted in the same order (invariant 2):
    // the rendered evidence line names the saved source and the profile.
    assert!(out.contains("saved investigation"), "out: {out}");
    assert!(out.contains("2 rows"), "out: {out}");
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn run_outcome_evidence_carries_the_connection_identity() {
    let root = temp_root("evidence-identity");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));

    let (outcome, out, err) = run_outcome(&repo, run_command(&id), &runtime, &state_db).await;
    assert_eq!(outcome.code, 0, "out: {out} err: {err}");
    let replay = outcome
        .replay
        .expect("a successful replay carries its evidence");
    assert_eq!(
        replay.evidence.connection_identity.as_deref(),
        Some(expected_identity(&runtime, "local").as_str()),
        "the evidence names the identity of the profile that actually ran"
    );
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn run_outcome_has_no_replay_on_stale_or_failure() {
    let root = temp_root("outcome-none");
    let runtime = runtime_at(&root, &single_profile_toml(&root));
    let repo = repo_at(&root);
    seed_events(&root.join("data.sqlite3")).await;
    let id = saved_id(&repo, &runtime).await;
    let state_db = SqliteStateStore::new(root.join("state.sqlite3"));

    // Stale: the document's revision moved on disk under the binding, so the
    // run is refused before anything executes and no replay exists.
    let document = fs::read_to_string(document_path(&root, &id)).unwrap();
    fs::write(
        document_path(&root, &id),
        document.replace("\"revision\": 1", "\"revision\": 2"),
    )
    .unwrap();
    let (stale, out, err) = run_outcome(&repo, run_command(&id), &runtime, &state_db).await;
    assert_eq!(stale.code, 2, "out: {out} err: {err}");
    assert!(stale.replay.is_none(), "a stale refusal carries no replay");
    assert!(err.contains("revision changed"), "err: {err}");

    // Execution failure: the SQL passed save's read-only shape gate but the
    // column does not exist, so the connector refuses it — the safety exit
    // carries no replay either.
    let (code, out, err) = run(
        &repo,
        InvestigationCommand::Save {
            name: "Broken".into(),
            description: None,
            sql: Some("SELECT nosuchcol FROM events".into()),
            file: None,
            connection: None,
            param_specs: Vec::new(),
        },
        &runtime,
    )
    .await;
    assert_eq!(code, 0, "save passed the shape gate: {out} err: {err}");
    let broken = out.lines().next().unwrap().to_string();
    let (failed, out, err) = run_outcome(&repo, run_command(&broken), &runtime, &state_db).await;
    assert_eq!(failed.code, 4, "out: {out} err: {err}");
    assert!(
        failed.replay.is_none(),
        "a failed execution carries no replay"
    );
    assert!(err.contains("nosuchcol"), "err: {err}");

    let _ = fs::remove_dir_all(root);
}
