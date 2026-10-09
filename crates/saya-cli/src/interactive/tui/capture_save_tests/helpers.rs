use super::*;

/// Creates the five-row table the envelope tests query: five rows through a
/// cap of two must report two rows AND truncation, so the asserted cap is
/// the one the query actually used. A dedicated table keeps each test's SQL
/// text unique — the worker's envelope stash is keyed by (profile, sql).
pub(super) fn seed_five_rows(path: &Path) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("seed runtime");
    runtime.block_on(async {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options)
            .await
            .expect("seed database creates");
        sqlx::query("CREATE TABLE t5 (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("seed table");
        for id in 1..=5 {
            sqlx::query("INSERT INTO t5 (id) VALUES (?)")
                .bind(id)
                .execute(&pool)
                .await
                .expect("seed row");
        }
        pool.close().await;
    });
}

pub(super) fn db_path(fx: &SaveRoundTripFixture) -> PathBuf {
    fx.root.0.join("capture.sqlite3")
}

/// Admits and spawns a dispatched task exactly as `loop_tick::pending` does.
/// The caller holds the permit-pool guard across the worker's life.
pub(super) fn spawn_real_sql_task(
    fx: &mut SaveRoundTripFixture,
    task: sql_task::SqlTask,
    _pool: &tokio::sync::MutexGuard<'static, ()>,
) {
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
}

/// Replaces the config behind BOTH runtime handles (the fixture's and the
/// app's) so the worker and the completion poll see the same runtime.
pub(super) fn set_max_rows(fx: &mut SaveRoundTripFixture, max_rows: usize) {
    let mut config = (*fx.runtime).clone();
    config.resolved.max_rows = max_rows;
    let fresh = Arc::new(config);
    fx.runtime = Arc::clone(&fresh);
    fx.app.runtime = fresh;
}
