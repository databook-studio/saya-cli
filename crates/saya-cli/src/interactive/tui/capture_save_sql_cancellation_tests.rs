use super::*;

#[test]
fn esc_after_spawn_cancels_dispatched_sql_and_discards_late_completion() {
    use crate::interactive::tui::application::SecondSqlDecision;
    use crate::interactive::tui::worker_permits::{running_workers, test_permit_lock};
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    let _pool = test_permit_lock();
    let mut fx = SaveRoundTripFixture::build_with_timeout(5);
    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    const SLOW: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt \
        WHERE x < 500000000) SELECT count(*) FROM cnt";
    let Dispatch::SqlTask(old_task) = fx.dispatch_line(&format!("/sql {SLOW}")) else {
        panic!("the real /sql dispatch creates a worker task");
    };
    let old_started = Instant::now();
    let old_cancel = CancellationToken::new();
    match fx.app.admit_second_sql() {
        SecondSqlDecision::Start(permit) => {
            fx.app.sql_task = Some((
                sql_task::spawn(
                    permit,
                    Arc::clone(&fx.runtime),
                    old_task.clone(),
                    old_cancel.clone(),
                ),
                old_task,
                old_started,
                old_cancel.clone(),
            ));
            fx.app.request.started = Some(old_started);
            fx.app.request.activity = Some("query".into());
        }
        SecondSqlDecision::Reject(message) => panic!("SQL admission refused: {message}"),
    }

    crate::interactive::tui::keys::handle_key(&mut fx.app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        old_cancel.is_cancelled(),
        "Esc signals the actual worker token"
    );
    assert!(fx.app.sql_task.is_none(), "Esc detaches the old receiver");

    let Dispatch::SqlTask(new_task) = fx.dispatch_line("/sql SELECT 7 AS value") else {
        panic!("a new direct query dispatches after detach");
    };
    let started = Instant::now();
    let new_cancel = CancellationToken::new();
    match fx.app.admit_second_sql() {
        SecondSqlDecision::Start(permit) => {
            fx.app.sql_task = Some((
                sql_task::spawn(
                    permit,
                    Arc::clone(&fx.runtime),
                    new_task.clone(),
                    new_cancel.clone(),
                ),
                new_task,
                started,
                new_cancel,
            ));
            fx.app.request.started = Some(started);
            fx.app.request.activity = Some("query".into());
        }
        SecondSqlDecision::Reject(message) => panic!("new SQL admission refused: {message}"),
    }
    tick_until(
        &mut fx,
        |fx| fx.app.captured.is_some(),
        "the new query did not complete after the old query detached",
    );

    let new_query = fx.app.last_query.as_ref().expect("new query is selectable");
    assert_eq!(new_query.sql, "SELECT 7 AS value");
    let new_capture = fx.app.captured.as_ref().expect("new query is captured");
    assert_eq!(new_capture.result.executed_sql, "SELECT 7 AS value");
    let deadline = Instant::now() + Duration::from_secs(5);
    while running_workers() != 0 {
        assert!(Instant::now() < deadline, "detached SQL worker settles");
        std::thread::yield_now();
    }
    assert!(
        old_started.elapsed() < Duration::from_secs(5),
        "the actual worker settles before the configured query timeout"
    );
    tick_workers(&mut fx.app, &fx.store, &mut fx.state);
    assert_eq!(
        fx.app.last_query.as_ref().map(|query| query.sql.as_str()),
        Some("SELECT 7 AS value"),
        "the old completion cannot replace the selectable query"
    );
    assert_eq!(
        fx.app
            .captured
            .as_ref()
            .map(|capture| capture.result.executed_sql.as_str()),
        Some("SELECT 7 AS value"),
        "the old completion cannot replace the capture"
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .all(|block| !block.text.contains(SLOW)),
        "detached SQL is not rendered later"
    );
    assert!(fx.app.sql_task.is_none());
    assert!(fx.app.request.started.is_none());
    assert!(fx.app.request.activity.is_none());
}
