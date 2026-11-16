use super::*;

/// The headless SQL boundary and the real TUI worker execute the same bounded
/// operation against the same SQLite database and produce the same rows.
#[test]
fn direct_sql_operation_matches_the_tui_worker_result() {
    let mut fx = SaveRoundTripFixture::build();
    let sql = "SELECT * FROM t";
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");
    let plain_result = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("analytics"),
            sql,
            false,
        ))
        .expect("the shared SQL operation succeeds");

    assert!(matches!(
        fx.dispatch_line("/connect analytics"),
        Dispatch::Handled
    ));
    let Dispatch::SqlTask(task) = fx.dispatch_line(&format!("/sql {sql}")) else {
        panic!("/sql dispatches its worker task");
    };
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
    tick_until(
        &mut fx,
        |fx| fx.app.captured.is_some(),
        "the /sql worker never completed into the capture",
    );

    let tui_result = &fx
        .app
        .captured
        .as_ref()
        .expect("successful query is captured")
        .result;
    assert_eq!(plain_result.columns, tui_result.columns);
    assert_eq!(plain_result.rows, tui_result.rows);
    assert_eq!(plain_result.row_count, tui_result.row_count);
    assert_eq!(plain_result.truncated, tui_result.truncated);
    assert_eq!(
        plain_result.row_count, 1,
        "the configured row cap is applied"
    );
    assert!(
        plain_result.truncated,
        "the cap reports more rows were available"
    );
}

#[test]
fn direct_sql_operation_preserves_profile_and_read_only_errors() {
    let fx = SaveRoundTripFixture::build();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");

    let no_profile = runtime
        .block_on(sql_operation::execute(&fx.runtime, None, "SELECT 1", false))
        .expect_err("a missing active profile is refused");
    assert_eq!(
        no_profile.to_string(),
        "No active profile. Use /connect <profile> first."
    );
    let no_profile_event = runtime.block_on(crate::interactive::tui::exec::run_sql(
        &fx.runtime,
        None,
        "SELECT 1",
        &CancellationToken::new(),
        &std::sync::Arc::new(std::sync::Mutex::new(None)),
    ));
    assert!(matches!(
        no_profile_event,
        crate::render::TerminalEvent::Error { message }
            if message == no_profile.to_string()
    ));
    let invalid_profile = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("missing"),
            "SELECT 1",
            false,
        ))
        .expect_err("an unknown profile is refused");
    assert!(invalid_profile.to_string().contains("not found"));
    let invalid_event = runtime.block_on(crate::interactive::tui::exec::run_sql(
        &fx.runtime,
        Some("missing"),
        "SELECT 1",
        &CancellationToken::new(),
        &std::sync::Arc::new(std::sync::Mutex::new(None)),
    ));
    assert!(matches!(
        invalid_event,
        crate::render::TerminalEvent::Error { message }
            if message == invalid_profile.to_string()
    ));

    let sql = "DELETE FROM t";
    let operation_error = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("analytics"),
            sql,
            false,
        ))
        .expect_err("direct SQL cannot write");
    let event = runtime.block_on(crate::interactive::tui::exec::run_sql(
        &fx.runtime,
        Some("analytics"),
        sql,
        &CancellationToken::new(),
        &std::sync::Arc::new(std::sync::Mutex::new(None)),
    ));
    let crate::render::TerminalEvent::Error { message } = event else {
        panic!("an unsafe query is rendered as an error");
    };
    assert_eq!(message, operation_error.to_string());
}

#[test]
fn a_pre_cancelled_sql_operation_stops_before_observable_connector_build() {
    let mut fx = SaveRoundTripFixture::build();
    Arc::make_mut(&mut fx.runtime).connections.profiles.insert(
        "unopened".into(),
        DatabaseProfile::Sqlite {
            path: ":memory:".into(),
            read_only: true,
        },
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");

    let uncancelled = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("unopened"),
            "SELECT 1",
            false,
        ))
        .expect_err("the synthetic in-memory profile reaches connector build");
    assert!(
        uncancelled
            .to_string()
            .contains("SQLite :memory: is not supported"),
        "the control observes the connector build boundary: {uncancelled}"
    );

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = runtime
        .block_on(sql_operation::execute_with_cancellation_and_source(
            &fx.runtime,
            Some("unopened"),
            "SELECT 1",
            false,
            &cancellation,
        ))
        .expect_err("a pre-cancelled operation stops before connector setup");

    assert_eq!(cancelled.to_string(), "query cancelled");
}

#[test]
fn direct_sql_operation_uses_the_configured_query_timeout() {
    const SLOW_QUERY: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt \
        WHERE x < 500000000) SELECT count(*) FROM cnt";
    let mut fx = SaveRoundTripFixture::build();
    std::sync::Arc::make_mut(&mut fx.runtime)
        .resolved
        .query_timeout_seconds = 1;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime builds");
    let error = runtime
        .block_on(sql_operation::execute(
            &fx.runtime,
            Some("analytics"),
            SLOW_QUERY,
            false,
        ))
        .expect_err("the configured one-second deadline interrupts the query");
    assert!(
        error.to_string().to_lowercase().contains("timed out"),
        "the operation preserves timeout errors: {error}"
    );
}
