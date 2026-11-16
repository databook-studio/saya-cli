use super::*;

/// When the turn finishes, the FIFO is cleared: no candidate survives into
/// the next turn.
#[test]
fn finishing_the_turn_clears_pending_candidates() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        StreamMsg::Done(Ok(AgentOutput {
            answer: "the answer".into(),
            events: Vec::new(),
            used_bounded_sql_query: false,
            tool_metadata: Vec::new(),
            usage: TokenUsage::new(0, 0),
            learning_usage: None,
            truncated: false,
            answer_sql: None,
        })),
    ]));
    assert!(app.drain_stream(&mut state), "the turn finished");
    assert!(
        app.pending_queries.is_empty(),
        "no candidate survives the finished turn"
    );
}

/// A failed completion consumes its candidate: a later completion cannot
/// promote a request whose own outcome already failed.
#[test]
fn failed_completion_consumes_its_candidate() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        completed("read-only database tool failed"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_sql(&app), None, "a failure never promotes");
    assert!(
        app.pending_queries.is_empty(),
        "the failed completion consumed its candidate"
    );
}

/// A denial consumes its candidate too: the front entry is popped without
/// promoting.
#[test]
fn denied_request_consumes_its_candidate() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![sql_request("SELECT 1", None), denied()]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_sql(&app), None);
    assert!(
        app.pending_queries.is_empty(),
        "the denied request's candidate was consumed"
    );
}

/// A fan-out completion neither promotes nor consumes the concrete candidate
/// waiting in the FIFO: only `bounded_sql_query` completions pair against it.
#[test]
fn fanout_completion_never_promotes_or_consumes() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        fanout_request("SELECT 1"),
        fanout_completed("4 rows"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_sql(&app), None, "fan-out is never selectable");
    assert_eq!(
        app.pending_queries.len(),
        1,
        "the concrete candidate was not consumed by the fan-out completion"
    );
    assert_eq!(
        app.pending_queries.front().map(|q| q.sql.as_str()),
        Some("SELECT 1")
    );
}
