//! The line front's end-to-end rows on the REAL `saya` binary, cross-platform.
//!
//! A pipe on stdin (no TTY) selects the line REPL, so nothing here needs a PTY,
//! a signal, or a shell: the child is spawned by
//! `harness::spawn_child_with_config_extra`, the provider is the scripted mock,
//! and every path is a `Path`/`PathBuf`. The binary comes from
//! `env!("CARGO_BIN_EXE_saya")`, the fixture from `tempfile`.
//!
//! Three facts:
//!
//! 1. a database question answers, and its rows came from the SQLite fixture
//!    through the read-only tool path — the `Using read-only tool:` line, the
//!    SQL it names, and the tool result the model was handed;
//! 2. a question whose turn outruns `[limits.database]` stops with the shared
//!    budget sentence, said once, and is not reported as a cancel;
//! 3. stdin EOF ends the front with exit code 0 — no Ctrl-C, no Ctrl-Break.
//!
//! This binary is not `cfg(unix)`, and the `windows job e2e` CI job runs it.

#[path = "chat_session_line_e2e/support.rs"]
mod support;

use std::time::Duration;

use support::{
    ANSWER, BUDGET_SENTENCE, QUESTION, SUM_SQL, ScriptStep, as_args, config, database_args,
    database_call, server, spawn_child_with_config_extra, sqlite_profile, stderr_of, stdout_of,
    tool_results, wait_for_stream, wait_for_stream_timeout,
};

#[test]
fn a_database_question_answers_from_the_read_only_query_path() {
    let dir = tempfile::tempdir().unwrap();
    let connections = sqlite_profile(dir.path());
    let mut mock = server(vec![
        database_call(),
        ScriptStep::ToolCall {
            name: "bounded_sql_query".into(),
            arguments: serde_json::json!({"sql": SUM_SQL}),
        },
        ScriptStep::Natural,
    ]);
    let args = database_args(&connections);
    let mut child =
        spawn_child_with_config_extra(&mock.address, &config(60), "line-e2e-db", &as_args(&args));
    child.send(QUESTION);
    wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    let stdout = stdout_of(&child);
    child.send(b"/exit\n");
    child.finish();
    mock.join();

    assert!(
        stdout.contains(ANSWER),
        "the database question was answered through the session: {stdout}"
    );
    assert!(
        stdout.contains("Using read-only tool: bounded_sql_query"),
        "the query ran on the read-only tool path: {stdout}"
    );
    assert!(
        stdout.contains(SUM_SQL),
        "the line named the SQL it ran: {stdout}"
    );
    let results = tool_results(&mock);
    assert!(
        results
            .iter()
            .any(|content| content.contains("30") && !content.contains("\"error\"")),
        "the query's own result (10 + 20) came back without an error: {results:?}"
    );
}

#[test]
fn a_turn_that_outruns_the_database_budget_stops_with_the_budget_sentence() {
    let dir = tempfile::tempdir().unwrap();
    let connections = sqlite_profile(dir.path());
    let mut mock = server(vec![
        // The turn's own first database use, then a request the source holds
        // past the armed ceiling: the deadline ends the turn, not a reply.
        database_call(),
        ScriptStep::DelayedNatural {
            delay: Duration::from_secs(15),
        },
        ScriptStep::Natural,
    ]);
    let args = database_args(&connections);
    let mut child = spawn_child_with_config_extra(
        &mock.address,
        &config(8),
        "line-e2e-budget",
        &as_args(&args),
    );
    child.send(QUESTION);
    wait_for_stream_timeout(
        &child.stderr_buf,
        BUDGET_SENTENCE,
        "the budget sentence",
        Duration::from_secs(40),
    );
    let stderr = stderr_of(&child);
    child.send(b"/exit\n");
    child.finish();
    mock.join();

    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("the run stopped on its"))
            .count(),
        1,
        "the sentence is said once: {stderr}"
    );
    assert!(
        !stderr.contains("Request cancelled."),
        "a budget stop is not a cancel: {stderr}"
    );
}

#[test]
fn stdin_eof_ends_the_line_front_with_exit_code_zero() {
    let mut mock = server(vec![ScriptStep::Natural]);
    let mut child = spawn_child_with_config_extra(&mock.address, &config(60), "line-e2e-eof", &[]);
    child.send(QUESTION);
    wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    // Nothing asks the front to quit: the closed pipe is the whole exit path.
    child.close_stdin();
    let status = child.exit();
    mock.join();

    assert_eq!(
        status.code(),
        Some(0),
        "a closed stdin ends the front cleanly, with no interrupt signal: {status:?}"
    );
}
