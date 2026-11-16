use super::*;

/// The snapshot writes the captured result itself: no `SqlTask` is
/// dispatched (nothing is re-run) and the file carries the captured
/// columns and rows exactly — not a viewport altered by scroll or
/// `/columns`.
#[test]
fn snapshot_export_never_queries() {
    let dir = temp_dir("snapshot-no-query");
    let path = dir.join("snap.csv");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let captured = captured_of(sample_result());
    let outcome = apply_export_action(
        snapshot_request(&path),
        &mut transcript,
        &mut state,
        &mut None,
        &captured,
    );
    assert!(
        !matches!(outcome, Some(Dispatch::SqlTask(_))),
        "a snapshot export must not dispatch a SqlTask"
    );
    let written = std::fs::read_to_string(&path).expect("snapshot wrote the file");
    assert_eq!(
        written, "id,name\n1,alice\n2,\"bob, jr\"\n",
        "the captured rows are exported exactly: {written}"
    );
    let msg = last_block(&transcript, BlockKind::System).expect("success is said");
    assert!(
        msg.contains("Exported 2 row(s) to")
            && msg.contains("from snapshot exec xabc-1")
            && msg.contains("(captured 22:13:20"),
        "the snapshot message names the execution and the capture time: {msg}"
    );
}

/// With nothing captured, the snapshot refuses and says how to recover —
/// it never falls back to re-running anything.
#[test]
fn snapshot_without_capture_says_how_to_recover() {
    let dir = temp_dir("snapshot-no-capture");
    let path = dir.join("snap.csv");
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", None, "model");
    let outcome = apply_export_action(
        snapshot_request(&path),
        &mut transcript,
        &mut state,
        &mut None,
        &None,
    );
    assert!(
        !matches!(outcome, Some(Dispatch::SqlTask(_))),
        "no capture, no task"
    );
    let msg = last_block(&transcript, BlockKind::Error).expect("the refusal is an error");
    assert!(
        msg.contains("No captured result to snapshot"),
        "the refusal names the mode: {msg}"
    );
    assert!(
        msg.contains("use /export --refresh"),
        "the refusal names the way out: {msg}"
    );
    assert!(!path.exists(), "nothing was written");
}

/// A refresh re-runs the last query on the connection it originally ran on
/// — `state.profile` must not override the last query's connection.
#[test]
fn refresh_export_preserves_original_connection() {
    let mut transcript = Transcript::new();
    let mut state = SessionState::new("test", Some("other".to_string()), "model");
    let mut last_query = Some(LastQuery {
        sql: "SELECT id FROM users".to_string(),
        connection: Some("analytics".to_string()),
    });
    // The legacy form (no mode flag) and the explicit --refresh form both
    // dispatch the same re-run.
    for mode in [None, Some(ExportMode::Refresh)] {
        let request = export_request(mode, false, "out.csv");
        let outcome =
            apply_export_action(request, &mut transcript, &mut state, &mut last_query, &None);
        let Some(Dispatch::SqlTask(task)) = outcome else {
            panic!("a refresh dispatches a SqlTask");
        };
        assert_eq!(
            task.profile,
            Some("analytics".to_string()),
            "the original connection is kept"
        );
        assert_eq!(task.sql, "SELECT id FROM users");
        let Followup::Export { request } = &task.followup else {
            panic!("the followup is the export");
        };
        assert_eq!(request.path, "out.csv");
        assert!(!request.overwrite);
        assert_eq!(request.mode, mode);
    }
}

/// The legacy refresh completion, when a capture exists, says the snapshot
/// mode would have exported the result the user already inspected; an
/// explicit `--refresh` never says it (the user chose the re-run).
#[test]
fn legacy_refresh_hint_names_snapshot_only_when_captured() {
    let dir = temp_dir("legacy-hint");
    // A distinct destination per call: without --overwrite the writer
    // refuses an existing file, which is the writer test's business, not
    // this one's.
    let run = |name: &str, mode: Option<ExportMode>, mut captured: Option<CapturedResult>| {
        let task = SqlTask::new(
            Some("analytics".to_string()),
            "SELECT id, name FROM users".to_string(),
            Followup::Export {
                request: export_request(mode, false, &dir.join(name).to_string_lossy()),
            },
        );
        let mut transcript = Transcript::new();
        complete(
            &task,
            TerminalEvent::QueryResult {
                result: sample_result(),
            },
            &mut transcript,
            &mut None,
            &mut captured,
        );
        transcript
    };

    // The legacy form with a capture present: the hint names --snapshot.
    let msg = last_block(
        &run("legacy.csv", None, captured_of(sample_result())),
        BlockKind::System,
    )
    .expect("export success is said");
    assert!(
        msg.contains("Exported 2 row(s) to")
            && msg.contains("— refreshed: re-ran the query just now"),
        "the refresh message names the re-run: {msg}"
    );
    assert!(
        msg.contains("(use --snapshot to export the result you already have)"),
        "the legacy form points at --snapshot when a capture exists: {msg}"
    );

    // An explicit --refresh with a capture present: no hint, the user chose.
    let msg = last_block(
        &run(
            "explicit.csv",
            Some(ExportMode::Refresh),
            captured_of(sample_result()),
        ),
        BlockKind::System,
    )
    .expect("export success is said");
    assert!(
        msg.contains("— refreshed: re-ran the query just now") && !msg.contains("--snapshot"),
        "an explicit refresh never suggests --snapshot: {msg}"
    );

    // The legacy form with nothing captured: no pointless hint.
    let msg = last_block(&run("bare.csv", None, None), BlockKind::System)
        .expect("export success is said");
    assert!(
        !msg.contains("--snapshot"),
        "no hint without a capture: {msg}"
    );
}
