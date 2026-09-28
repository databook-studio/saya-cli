//! S5/D7: only a successful, concrete `bounded_sql_query` completion becomes
//! the latest selectable query. A request is a candidate on a bounded FIFO;
//! the completion decides promotion; a failure or a denial never promotes;
//! fan-out never becomes selectable. Driven through `drain_stream`, the same
//! entry point the loop tick uses.

use super::*;

fn app_and_state() -> (App, SessionState) {
    (idle_app(), SessionState::new("s1", None, "test-model"))
}

/// A stream that carries exactly `messages`, as the agent thread would.
fn stream_with(messages: Vec<StreamMsg>) -> Stream {
    let (tx, rx) = unbounded_channel();
    for message in messages {
        let _ = tx.send(message);
    }
    Stream {
        rx,
        cancel: CancellationToken::new(),
        prompt: "question".into(),
    }
}

/// A `bounded_sql_query` request event with the given sql and optional
/// connection argument.
fn sql_request(sql: &str, connection: Option<&str>) -> StreamMsg {
    let mut arguments = serde_json::json!({ "sql": sql });
    if let Some(connection) = connection {
        arguments["connection"] = serde_json::json!(connection);
    }
    StreamMsg::Event(AgentEvent::tool_requested(
        "bounded_sql_query",
        arguments,
        None,
    ))
}

/// A `bounded_sql_query_all` request event.
fn fanout_request(sql: &str) -> StreamMsg {
    StreamMsg::Event(AgentEvent::tool_requested(
        "bounded_sql_query_all",
        serde_json::json!({ "sql": sql }),
        None,
    ))
}

/// A completion for `bounded_sql_query`: `summary` decides failure the same
/// way the renderers classify it (the "failed" substring).
fn completed(summary: &str) -> StreamMsg {
    StreamMsg::Event(AgentEvent::ToolCompleted {
        name: "bounded_sql_query".into(),
        summary: summary.into(),
    })
}

/// A completion for `bounded_sql_query_all`.
fn fanout_completed(summary: &str) -> StreamMsg {
    StreamMsg::Event(AgentEvent::ToolCompleted {
        name: "bounded_sql_query_all".into(),
        summary: summary.into(),
    })
}

/// A denial of `bounded_sql_query`: no `ToolCompleted` follows it.
fn denied() -> StreamMsg {
    StreamMsg::Event(AgentEvent::ToolDenied {
        name: "bounded_sql_query".into(),
        reason: "user denied the call".into(),
    })
}

fn selectable_sql(app: &App) -> Option<&str> {
    app.last_query.as_ref().map(|q| q.sql.as_str())
}

fn selectable_connection(app: &App) -> Option<&str> {
    app.last_query
        .as_ref()
        .and_then(|q| q.connection.as_deref())
}

/// The unchanged success path: a successful completion promotes its request
/// to the selectable query.
#[test]
fn successful_completion_promotes_its_request() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 1"),
        "the successful completion promoted its request"
    );
}

/// The headline behaviour: a request followed by a failure completion must
/// not overwrite the previous selectable query.
#[test]
fn failed_request_does_not_replace_successful_query() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_sql(&app), Some("SELECT 1"));

    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 2", None),
        completed("read-only database tool failed"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 1"),
        "a failed completion must leave the previous selectable query in place"
    );
}

/// A denied call never ran: its request must not become the selectable
/// query, and a later unmatched completion cannot promote anything stale.
#[test]
fn denied_request_does_not_replace_successful_query() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);

    app.request.stream = Some(stream_with(vec![sql_request("SELECT 2", None), denied()]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 1"),
        "a denial never promotes its request"
    );

    // The FIFO holds nothing after the denial: a completion without a
    // request cannot promote anything.
    app.request.stream = Some(stream_with(vec![completed("2 rows")]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_sql(&app), Some("SELECT 1"));
}

/// A request alone is never selectable: the candidate waits for its
/// completion.
#[test]
fn requested_but_not_completed_is_not_selectable() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![sql_request("SELECT 1", None)]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        None,
        "a request is a candidate, not a selectable query"
    );
}

/// Fan-out (`bounded_sql_query_all`) never becomes selectable, even when its
/// completion reports success: the user picks one connection with /sql.
#[test]
fn fanout_requires_concrete_result_selection() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        fanout_request("SELECT 1"),
        fanout_completed("4 rows"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        None,
        "fan-out is never selectable, success or not"
    );
}

/// Batch pairing in order: requests A, B then completions ok(A), fail(B)
/// promote A only — the FIFO pairs requests with completions by arrival.
#[test]
fn batch_requests_pair_in_order() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        sql_request("SELECT 2", None),
        completed("1 row"),
        completed("read-only database tool failed"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_sql(&app),
        Some("SELECT 1"),
        "A's success promoted; B's failure did not"
    );
}

/// The connection argument is carried through promotion. An absent-or-empty
/// argument means "no connection named" — the executor's own semantic — and
/// resolves to the session's connection at request time (S9 invariant 4),
/// which for a profile-less session is nothing at all.
#[test]
fn connection_argument_preserved_and_empty_resolves_to_the_session() {
    let (mut app, mut state) = app_and_state();
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", Some("analytics")),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_connection(&app), Some("analytics"));

    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 2", Some("")),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_sql(&app), Some("SELECT 2"));
    assert_eq!(
        selectable_connection(&app),
        None,
        "empty means no connection named, and this session has none to fill with"
    );

    // With a session profile, the same empty argument resolves to it.
    state.profile = Some("analytics".into());
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 3", Some("")),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_connection(&app),
        Some("analytics"),
        "empty means no connection named: the session's connection fills it"
    );
}

/// S9 invariant 4: a `bounded_sql_query` with no `connection` argument runs
/// on the session's active profile, so the promoted `LastQuery` carries it —
/// `/investigation save` then saves the connection that actually ran it.
#[test]
fn unnamed_query_promotes_the_sessions_active_profile() {
    let (mut app, mut state) = app_and_state();
    state.profile = Some("analytics".into());
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_connection(&app),
        Some("analytics"),
        "an unnamed query ran on the session's active profile"
    );
}

/// With no active profile selected, the session runs on the runtime's
/// resolved default — the connection an unnamed query actually used.
#[test]
fn unnamed_query_promotes_the_sessions_default_profile() {
    let (mut app, mut state) = app_and_state();
    let mut runtime = unused_runtime();
    runtime.resolved.profile_name = Some("prod".into());
    app.runtime = Arc::new(runtime);
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", None),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_connection(&app),
        Some("prod"),
        "an unnamed query ran on the session's default profile"
    );
    // The active profile wins when one is selected.
    state.profile = Some("analytics".into());
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 2", None),
        completed("2 rows"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(selectable_connection(&app), Some("analytics"));
}

/// A named connection argument still stands as stated, even when the session
/// has an active profile of its own.
#[test]
fn named_connection_argument_still_wins_over_the_session_profile() {
    let (mut app, mut state) = app_and_state();
    state.profile = Some("analytics".into());
    app.request.stream = Some(stream_with(vec![
        sql_request("SELECT 1", Some("prod")),
        completed("1 row"),
    ]));
    app.drain_stream(&mut state);
    assert_eq!(
        selectable_connection(&app),
        Some("prod"),
        "the model's named connection is the fact"
    );
}

/// The FIFO is bounded: the 33rd request drops the oldest candidate.
#[test]
fn pending_candidates_are_bounded_at_32_dropping_the_oldest() {
    let (mut app, mut state) = app_and_state();
    let messages: Vec<StreamMsg> = (1..=33)
        .map(|n| sql_request(&format!("SELECT {n}"), None))
        .collect();
    app.request.stream = Some(stream_with(messages));
    app.drain_stream(&mut state);
    assert_eq!(
        app.pending_queries.len(),
        32,
        "the FIFO holds at most 32 candidates"
    );
    assert_eq!(
        app.pending_queries.front().map(|q| q.sql.as_str()),
        Some("SELECT 2"),
        "the oldest candidate was dropped"
    );
}

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
