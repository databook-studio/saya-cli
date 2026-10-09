/// Seeds the events table straight into the profile's sqlite file, so a
/// replay has something bounded to read.
async fn seed_events(database: &Path) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(database)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE events (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO events (id, label) VALUES (1, 'first'), (2, 'second')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// Drives the dispatcher's typed-outcome seam with output captured, the way
/// the TUI replay adapter consumes a run (D12): the outcome, not just the
/// exit code.
async fn run_outcome(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    state_db: &SqliteStateStore,
) -> (RunOutcome, String, String) {
    capture_output_start();
    let outcome = super::run_investigation_outcome_controlled_in(
        repo,
        command,
        runtime,
        RenderFormat::Text,
        false,
        state_db,
        super::RunControl {
            cancellation: None,
            machine_driven: false,
        },
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    (outcome, out, err)
}

async fn run_outcome_with_cancellation(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    state_db: &SqliteStateStore,
    cancellation: &CancellationToken,
) -> (RunOutcome, String, String) {
    capture_output_start();
    let outcome = super::run_investigation_outcome_controlled_in(
        repo,
        command,
        runtime,
        RenderFormat::Text,
        false,
        state_db,
        super::RunControl {
            cancellation: Some(cancellation),
            machine_driven: false,
        },
    )
    .await
    .unwrap();
    let (out, err) = capture_output_take();
    (outcome, out, err)
}

fn run_command(id: &str) -> InvestigationCommand {
    InvestigationCommand::Run {
        id: id.to_string(),
        connection: None,
        revalidate: false,
        report: None,
        rows: None,
        overwrite: false,
        params: Vec::new(),
    }
}

/// Drives the machine-driven seam the MCP replay adapter uses: the same
/// typed operation, explicitly machine-driven, with output captured.
async fn run_outcome_machine_driven(
    repo: &InvestigationRepository,
    command: InvestigationCommand,
    runtime: &RuntimeConfig,
    state_db: &SqliteStateStore,
    cancellation: &CancellationToken,
) -> (RunOutcome, String, String) {
    capture_output_start();
    let outcome = command
        .run_outcome_machine_driven_with_cancellation(
            repo,
            runtime,
            RenderFormat::Text,
            false,
            state_db,
            cancellation,
        )
        .await
        .unwrap();
    let (out, err) = capture_output_take();
    (outcome, out, err)
}
