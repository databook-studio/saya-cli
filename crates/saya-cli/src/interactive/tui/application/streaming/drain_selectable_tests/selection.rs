use super::*;

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
