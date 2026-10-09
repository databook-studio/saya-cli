/// Once the pending FIFO overflows, the turn is desynchronised: no capture
/// is queued, no completion pairs anything, and the capture slot is exactly
/// as it was.
#[test]
fn desync_promotes_no_capture() {
    let (mut app, mut state) = app_and_state();
    app.captured = Some(prev_capture());
    let mut messages: Vec<StreamMsg> = (1..=33)
        .map(|n| sql_request(&format!("SELECT {n}"), Some("analytics")))
        .collect();
    messages.push(StreamMsg::QueryCaptured(capture_of(
        "SELECT 33",
        "analytics",
        1,
    )));
    messages.push(completed("1 row"));
    app.request.stream = Some(stream_with(messages));
    app.drain_stream(&mut state);
    assert!(app.pending_queries_desync, "the overflow desynced the turn");
    let kept = app
        .captured
        .as_ref()
        .expect("a desynchronised turn promotes no capture and clears nothing");
    assert_eq!(kept.result.executed_sql, "SELECT 0");
    assert!(
        app.agent_captures.is_empty(),
        "the capture queue was cleared on desync"
    );
    assert_eq!(app.agent_captures.gap, None);
}

/// R4-9: a TurnReset discards the failed attempt's whole pairing state — the
/// pending FIFO and the unmatched-capture queue — so the retry's completion
/// can only ever promote the retry's own request and pair the retry's own
/// capture, never a stale candidate from the attempt that was thrown away.
#[test]
fn a_turn_reset_discards_the_discarded_attempt_s_pairing_state() {
    let (mut app, mut state) = app_and_state();
    let retry_capture = capture_of("SELECT 2", "analytics", 2);
    let retry_result = retry_capture.result.clone();
    app.request.stream = Some(stream_with(vec![
        // The discarded attempt: its request and its queued capture.
        sql_request("SELECT 1", Some("analytics")),
        StreamMsg::QueryCaptured(capture_of("SELECT 1", "analytics", 1)),
        // The attempt failed mid-stream; the turn retries.
        StreamMsg::Event(AgentEvent::turn_reset()),
        // The retry: its own request, its own capture, its completion.
        sql_request("SELECT 2", Some("analytics")),
        StreamMsg::QueryCaptured(retry_capture),
        completed("2 rows"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 2"),
        "the retry's completion promotes the retry's own request, never the \
         discarded attempt's stale candidate"
    );
    let held = app
        .captured
        .as_ref()
        .expect("the retry's completion pairs the retry's own capture");
    assert_eq!(
        held.result, retry_result,
        "the slot holds the retry's capture, never the discarded attempt's"
    );
    assert!(
        app.pending_queries.is_empty(),
        "the retry consumed its own candidate"
    );
}

/// The queue never survives a turn: an unmatched capture (its completion
/// never came) is dropped when the turn ends.
#[test]
fn turn_end_clears_unmatched_captures() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", Some("analytics")),
        StreamMsg::QueryCaptured(capture_of("SELECT 1", "analytics", 1)),
        done(),
    ]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert!(app.captured.is_none(), "no pairing, no capture");
    assert!(
        app.agent_captures.is_empty(),
        "the turn end cleared the unmatched capture"
    );
    assert_eq!(app.agent_captures.gap, None, "no promotion, no gap");
}
