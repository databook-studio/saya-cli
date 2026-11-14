/// A failed replay pushes an error block and leaves the previous capture and
/// selectable query exactly as they were.
#[test]
fn failed_replay_keeps_previous_capture() {
    let mut app = idle_app();
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(temp_dir("failed").as_path());
    let previous = CapturedResult {
        result: sample_result(),
        evidence: saved_evidence(),
    };
    app.captured = Some(CapturedResult {
        result: previous.result.clone(),
        evidence: previous.evidence.clone(),
    });
    app.last_query = Some(LastQuery {
        sql: "SELECT 2 AS two".to_string(),
        connection: Some("other".to_string()),
    });
    let (tx, rx) = mpsc::channel();
    tx.send(done(2, "no investigation nope", None))
        .expect("pre-delivered failure");
    put_running_replay(&mut app, rx, "nope");

    tick_workers(&mut app, &store, &mut state);

    let block = last_block(&app.transcript, BlockKind::Error).expect("the failure is said");
    assert_eq!(block, "no investigation nope");
    let captured = app
        .captured
        .as_ref()
        .expect("the previous capture survives");
    assert_eq!(
        captured.result.executed_sql, previous.result.executed_sql,
        "a failed replay never replaces the capture"
    );
    let last_query = app
        .last_query
        .as_ref()
        .expect("the previous query survives");
    assert_eq!(last_query.sql, "SELECT 2 AS two");
    assert_eq!(last_query.connection.as_deref(), Some("other"));
}

/// A non-zero exit that still carries the typed replay — a successful
/// execution whose later step refused, unreachable from the TUI's slash run
/// (which passes no report flags) — keeps today's behaviour: the text as an
/// error block, and the replay still promoted and captured, since the query
/// did run.
#[test]
fn failed_code_with_typed_replay_still_promotes_and_captures() {
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured: Option<CapturedResult> = None;
    complete(
        done(2, "the report write refused", Some(replay())),
        accounted_bytes(&sample_result()),
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    let block = last_block(&transcript, BlockKind::Error).expect("the failure is said");
    assert_eq!(block, "the report write refused");
    assert!(
        last_block(&transcript, BlockKind::System).is_none(),
        "no system block on a failure: {:?}",
        transcript.blocks()
    );
    let last_query = last_query
        .as_ref()
        .expect("the run's replay is still promoted");
    assert_eq!(last_query.sql, "SELECT 1 AS one");
    assert_eq!(last_query.connection.as_deref(), Some("local"));
    assert!(captured.is_some(), "the run's result is still captured");
}

/// A successful replay whose captured output and stderr are both empty
/// pushes no block: an empty System block is a transcript glitch, not a
/// message.
#[test]
fn successful_replay_with_no_output_pushes_no_block() {
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured: Option<CapturedResult> = None;
    complete(
        done(0, "", None),
        None,
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    assert!(
        transcript.blocks().is_empty(),
        "an empty output is no block at all: {:?}",
        transcript.blocks()
    );
    assert!(last_query.is_none() && captured.is_none());
}

/// The same skip for a failure: empty text is no block of any kind — the
/// completion never pushes a block it has nothing to say in.
#[test]
fn failed_replay_with_no_output_pushes_no_block() {
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured: Option<CapturedResult> = None;
    complete(
        done(2, "", None),
        None,
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    assert!(
        transcript.blocks().is_empty(),
        "an empty failure text is no block at all: {:?}",
        transcript.blocks()
    );
}
