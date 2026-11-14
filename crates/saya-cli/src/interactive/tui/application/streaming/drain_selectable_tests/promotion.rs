use super::*;

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
