use super::*;

/// The contract at its strongest boundary: after a real `/sql SELECT * FROM t`
/// completes into `App.captured` through the worker path, the real
/// `queue_session_save` write lands in the session dir without the captured
/// row value. The mutation proof (a captured-result copy into a persisted
/// turn, applied then reverted) turns the final assertion red.
#[test]
fn a_captured_sql_result_never_reaches_the_saved_session() {
    let mut fx = SaveRoundTripFixture::build();
    let sentinel = sentinel();

    // The real dispatch chain: select the profile, then run the query.
    let connected = fx.dispatch_line("/connect analytics");
    assert!(
        matches!(connected, Dispatch::Handled),
        "the connect is handled"
    );
    assert_eq!(
        fx.state.profile.as_deref(),
        Some("analytics"),
        "the profile is selected"
    );
    let outcome = fx.dispatch_line("/sql SELECT * FROM t");
    let Dispatch::SqlTask(task) = outcome else {
        panic!("/sql dispatches its worker task");
    };

    // The real admission + spawn, exactly as `loop_tick::pending` does.
    let _pool = test_permit_lock();
    match fx.app.admit_second_sql() {
        SecondSqlDecision::Start(permit) => {
            let started = Instant::now();
            let cancellation = CancellationToken::new();
            fx.app.sql_task = Some((
                sql_task::spawn(
                    permit,
                    Arc::clone(&fx.runtime),
                    task.clone(),
                    cancellation.clone(),
                ),
                task,
                started,
                cancellation,
            ));
            fx.app.request.started = Some(started);
            fx.app.request.activity = Some("query".into());
        }
        SecondSqlDecision::Reject(message) => panic!("admission refused the query: {message}"),
    }

    // The real worker poll: the completion lands in the capture.
    tick_until(
        &mut fx,
        |fx| fx.app.captured.is_some(),
        "the /sql worker never completed into the capture",
    );
    let capture = fx
        .app
        .captured
        .as_ref()
        .expect("a successful /sql is captured");
    let cell = capture.result.rows[0].as_array().expect("a row array")[0]
        .as_str()
        .expect("a text cell");
    assert_eq!(cell, sentinel, "the capture holds the queried row value");
    assert!(
        !capture.result.executed_sql.contains(&sentinel),
        "the sentinel must be a row value, not part of the SQL: {}",
        capture.result.executed_sql
    );

    // The real save boundary, then the real poll until it lands.
    queue_session_save(&mut fx.app, &fx.store, &fx.state);
    tick_until(
        &mut fx,
        |fx| fx.app.session_save.is_none() && fx.app.pending_session_save.is_none(),
        "the session save never completed",
    );
    let saved = std::fs::read_to_string(
        fx.root
            .0
            .join("session-store")
            .join(format!("{SESSION_ID}.json")),
    )
    .expect("the queued save wrote the session file");
    assert!(
        saved.contains(SESSION_ID),
        "our session really was written: {saved}"
    );
    assert!(
        !saved.contains(&sentinel),
        "captured result rows reached the saved session: {saved}"
    );
}
