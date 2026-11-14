/// Every registered slash command survives the real dispatch: the chain
/// must never panic on an unowned action. Each command gets a fresh
/// fixture and its representative line; the assertion is that `dispatch`
/// returns — whatever outcome it picks.
#[test]
fn every_registered_slash_command_survives_dispatch() {
    let root = std::env::temp_dir().join(format!(
        "saya-chain-lines-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("chain test lines root");
    let mut failed: Vec<String> = Vec::new();
    for name in KNOWN_COMMANDS {
        let line = representative_line(name, &root);
        let tag = name.replace('-', "_");
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let mut fx = ChainFixture::build(&tag);
            dispatch_line(&line, &mut fx)
        }));
        if let Err(payload) = outcome {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("<non-string panic>");
            failed.push(format!("/{name} panicked ({line}): {message}"));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        failed.is_empty(),
        "{} registered command(s) panicked in dispatch:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

fn sample_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".to_string(), "name".to_string()],
        rows: vec![
            serde_json::json!([1, "sentinel-alice"]),
            serde_json::json!([2, "bob"]),
        ],
        row_count: 2,
        truncated: false,
        executed_sql: "SELECT id, name FROM users".to_string(),
    }
}

fn captured_of(result: QueryResult) -> Option<CapturedResult> {
    let evidence = ExecutionEvidence::for_result(
        &result,
        ExecutionEvidenceArgs {
            execution_id: "xabc-1".to_string(),
            connection_label: "demo".to_string(),
            connection_identity: None,
            dialect: SqlDialect::Sqlite,
            max_rows: 100,
            started_unix_ms: STARTED_UNIX_MS,
            finished_unix_ms: STARTED_UNIX_MS + 2_000,
            source: EvidenceSource::DirectSql,
        },
    );
    Some(CapturedResult { result, evidence })
}

fn last_block(transcript: &Transcript, kind: BlockKind) -> Option<String> {
    transcript
        .blocks()
        .iter()
        .rev()
        .find(|block| block.kind == kind)
        .map(|block| block.text.clone())
}

#[test]
fn run_resume_dispatch_keeps_ordinary_and_explicit_retry_shell_commands_distinct() {
    let mut ordinary = ChainFixture::build("resume-ordinary");
    assert!(matches!(
        dispatch_line("/run resume run-ordinary", &mut ordinary),
        Dispatch::Handled
    ));
    let ordinary_hint = last_block(&ordinary.app.transcript, BlockKind::System).unwrap();
    assert!(ordinary_hint.contains("saya run resume run-ordinary`"));
    assert!(!ordinary_hint.contains("--retry-incomplete"));

    let mut retry = ChainFixture::build("resume-retry");
    assert!(matches!(
        dispatch_line("/run resume run-retry --retry-incomplete", &mut retry),
        Dispatch::Handled
    ));
    let retry_hint = last_block(&retry.app.transcript, BlockKind::System).unwrap();
    assert!(retry_hint.contains("saya run resume run-retry --retry-incomplete`"));
}

/// `/sql` (its task returned untouched) then `/export --snapshot`: the
/// snapshot writes the captured result through the real dispatch, with no
/// query dispatched at all.
#[test]
fn export_snapshot_after_sql_writes_the_file_through_dispatch() {
    let mut fx = ChainFixture::build("export-snapshot");
    fx.app.captured = captured_of(sample_result());
    let outcome = dispatch_line("/sql SELECT 1", &mut fx);
    assert!(
        matches!(outcome, Dispatch::SqlTask(_)),
        "/sql dispatches its task"
    );
    let path = fx.root.0.join("snapshot.csv");
    let outcome = dispatch_line(&format!("/export --snapshot {}", path.display()), &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the snapshot export is handled inline"
    );
    let written = std::fs::read_to_string(&path).expect("the snapshot file was written");
    assert!(
        written.contains("sentinel-alice"),
        "the rows come from the capture: {written}"
    );
    let msg = last_block(&fx.app.transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Exported") && msg.contains("snapshot exec"),
        "the success names the capture: {msg}"
    );
}

/// `/investigation save` with neither --sql nor --file fills from the last
/// query through the real dispatch, saves into the state db, and
/// `/investigation list` lists it back through the same chain (bare
/// `/investigations` opens the picker overlay instead, so the text listing
/// is named explicitly here).
#[test]
fn investigation_save_through_dispatch_saves() {
    let mut fx = ChainFixture::build("investigation-save");
    fx.app.last_query = Some(LastQuery {
        sql: "SELECT 1".to_string(),
        connection: Some("demo".to_string()),
    });
    let outcome = dispatch_line("/investigation save chain-sentinel", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "an inline save reports handled"
    );
    let msg = last_block(&fx.app.transcript, BlockKind::System).expect("the save said something");
    assert!(
        msg.contains("Saved exactly as shown"),
        "the save succeeded: {msg}"
    );
    let outcome = dispatch_line("/investigation list", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the listing reports handled"
    );
    let list = last_block(&fx.app.transcript, BlockKind::System).expect("the listing is said");
    assert!(
        list.contains("chain-sentinel"),
        "the saved investigation is listed: {list}"
    );
}

/// Bare `/investigations` opens the picker overlay in the TUI: the dispatch
/// returns before the text listing would run, and the overlay opens through
/// the same chain when the loop applies the outcome.
#[test]
fn bare_investigations_opens_the_picker() {
    let mut fx = ChainFixture::build("investigation-picker");
    let outcome = dispatch_line("/investigations", &mut fx);
    assert!(
        matches!(outcome, Dispatch::OpenInvestigationPicker),
        "bare /investigations opens the picker"
    );
    // Any tail keeps the text listing path — the parser handles it.
    let outcome = dispatch_line("/investigations --list", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "--list keeps the text listing"
    );
}
