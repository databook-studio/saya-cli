use super::*;

#[tokio::test]
async fn a_runaway_statement_is_interrupted_and_the_connection_is_released() {
    let run = TempRun::new("timeout");
    let tool = admitted(&run).with_query_timeout(Duration::from_millis(500));
    let error = tool
        .run("SELECT count(*) FROM range(100000000) a, range(100000000) b")
        .await
        .expect_err("the cross product never finishes; the ceiling must fire");
    assert!(matches!(error, ScratchError::TimedOut), "{error}");
    // The await-on-cancel proved the query released the mutex: the connection
    // still serves.
    let answer = tool.run("SELECT 42 AS answer").await.unwrap();
    assert_eq!(answer.rows, json_rows(json!([[42]])));
}

/// A cancelled turn stops a running query: the engine is interrupted, the
/// blocking task is joined (so the connection mutex is released), and the
/// tool boundary surfaces the cancelled detail — never a timeout. The same
/// tool then still serves a later statement.
#[tokio::test]
async fn cancelling_a_runaway_scratch_query_interrupts_it_and_frees_the_connection() {
    let run = TempRun::new("cancel");
    let tool = admitted(&run);
    let token = CancellationToken::new();
    let canceller = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            token.cancel();
        })
    };
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        Duration::from_secs(15),
        tool.execute_with_context(
            "scratch_sql",
            json!({"sql": "SELECT count(*) FROM range(100000000) a, range(100000000) b"}),
            context(&token),
        ),
    )
    .await
    .expect("the guard: a hung query means the interrupt-and-join regressed")
    .expect_err("a cancelled call is an error, never a result");
    let elapsed = started.elapsed();
    canceller.await.unwrap();
    match outcome {
        ToolError::QueryFailedDetail(detail) => {
            assert!(
                detail.contains("cancelled"),
                "the cancelled detail: {detail}"
            );
            assert!(!detail.contains("timed out"), "never a timeout: {detail}");
        }
        other => panic!("cancellation must surface as the cancelled detail, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(5),
        "cancellation must land well under the 30 s ceiling, took {elapsed:?}"
    );
    // The join proved the query released the mutex: the connection still
    // serves on the same tool.
    let answer = tokio::time::timeout(Duration::from_secs(15), tool.run("SELECT 42 AS answer"))
        .await
        .expect("the follow-up guard")
        .unwrap();
    assert_eq!(answer.rows, json_rows(json!([[42]])));
}

/// A token already cancelled before the call starts the statement not at
/// all: nothing reaches DuckDB, and the effect the statement would have had
/// — the table it would have created — is absent afterwards.
#[tokio::test]
async fn a_precancelled_turn_never_starts_a_scratch_statement() {
    let run = TempRun::new("precancel");
    let tool = admitted(&run);
    let token = CancellationToken::new();
    token.cancel();
    let error = tokio::time::timeout(
        Duration::from_secs(15),
        tool.execute_with_context(
            "scratch_sql",
            json!({"sql": "CREATE TABLE should_not_exist AS SELECT 1"}),
            context(&token),
        ),
    )
    .await
    .expect("the guard")
    .expect_err("a pre-cancelled call is an error, never a result");
    assert!(
        matches!(&error, ToolError::QueryFailedDetail(detail) if detail.contains("cancelled")),
        "{error}"
    );
    // The statement never started: the table is absent — a catalog error,
    // not a count.
    let missing = tokio::time::timeout(
        Duration::from_secs(15),
        tool.run("SELECT count(*) FROM should_not_exist"),
    )
    .await
    .expect("the follow-up guard")
    .expect_err("the cancelled turn's statement must have had no effect");
    match missing {
        ScratchError::Execution { message } => {
            assert!(message.contains("Catalog Error"), "{message}");
        }
        other => panic!("the table must be absent, got {other:?}"),
    }
}

/// An uncancelled context runs the statement normally: the context path
/// returns exactly what the plain `run` path returns — the cancellation
/// wiring cannot change the outcome of a turn that is never cancelled.
#[tokio::test]
async fn an_uncancelled_context_runs_the_statement_normally() {
    let run = TempRun::new("uncancelled");
    let tool = admitted(&run);
    let token = CancellationToken::new();
    let sql = "SELECT 41 + 1 AS answer";
    let through_context = tokio::time::timeout(
        Duration::from_secs(15),
        tool.execute_with_context("scratch_sql", json!({"sql": sql}), context(&token)),
    )
    .await
    .expect("the guard")
    .unwrap();
    let plain = tokio::time::timeout(Duration::from_secs(15), tool.run(sql))
        .await
        .expect("the follow-up guard")
        .unwrap();
    assert_eq!(
        through_context,
        serde_json::to_value(plain).unwrap(),
        "the context path must equal the plain path"
    );
}

/// The per-statement ceiling is unchanged by the cancellation wiring: a tiny
/// ceiling with an uncancelled token still reports a timeout at the tool
/// boundary, not a cancellation.
#[tokio::test]
async fn timeout_still_reports_timeout_not_cancellation() {
    let run = TempRun::new("timeout-not-cancel");
    let tool = admitted(&run).with_query_timeout(Duration::from_millis(500));
    let token = CancellationToken::new();
    let error = tokio::time::timeout(
        Duration::from_secs(15),
        tool.execute_with_context(
            "scratch_sql",
            json!({"sql": "SELECT count(*) FROM range(100000000) a, range(100000000) b"}),
            context(&token),
        ),
    )
    .await
    .expect("the guard")
    .expect_err("the runaway must hit the ceiling");
    assert_eq!(error, ToolError::QueryTimedOut, "{error}");
}
