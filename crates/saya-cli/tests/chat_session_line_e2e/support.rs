//! The setup the line front's end-to-end rows share: the scripted-provider
//! harness module, the SQLite fixture, the child's config and arguments, and
//! the readers a row asserts on.
//!
//! Every path here is a `Path`/`PathBuf` over a `tempfile` directory and every
//! call is plain `std` — no PTY, no `/bin` path, no signal — so the same rows
//! run in the Windows CI job.

#[path = "../mvp/session_question_budget/harness.rs"]
#[allow(dead_code)] // shared harness: these rows use part of it
pub(super) mod harness;

#[path = "../support/sqlite_fixture.rs"]
#[allow(dead_code)] // shared fixture: only the database rows need it
mod sqlite_fixture;

use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

pub(super) use harness::{
    ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream,
    wait_for_stream_timeout,
};

pub(super) const QUESTION: &[u8] = b"how much revenue is there\n";
pub(super) const ANSWER: &str = "the-final-answer";
pub(super) const SUM_SQL: &str = "SELECT SUM(amount) AS total FROM revenue";

/// The budget sentence a stopped turn says on stderr, in the current words.
pub(super) const BUDGET_SENTENCE: &[u8] =
    b"the run stopped on its elapsed-time budget; any answer it produced is partial or absent";

/// A mock server with a fresh script and no held response.
pub(super) fn server(script: Vec<ScriptStep>) -> harness::MockServer {
    spawn_script_server(listener(), script, Arc::new(AtomicBool::new(false)))
}

fn listener() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

/// The one config every row states: sharing is the operator's consent (the mock
/// is not a local endpoint), memory is off so no extraction adds a request, and
/// the row states its own database ceiling.
pub(super) fn config(database_elapsed_seconds: u64) -> String {
    format!(
        "[ai]\nallow_data_sharing = true\n\n\
         [limits.database]\nelapsed_seconds = {database_elapsed_seconds}\n\n\
         [memory]\nmode = 'off'\n"
    )
}

/// A SQLite file under the test's own temp directory plus the `connections.toml`
/// that names it as the read-only profile `local`. Returns that connections file.
pub(super) fn sqlite_profile(dir: &Path) -> PathBuf {
    let database = dir.join("golden.sqlite");
    sqlite_fixture::create(
        &database,
        &[
            "CREATE TABLE revenue (amount INTEGER);",
            "INSERT INTO revenue (amount) VALUES (10), (20);",
        ],
    );
    let connections = dir.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    connections
}

/// The arguments a database row passes: the profile, and `bypass` because a
/// piped REPL cannot answer the database tools' own approval prompt.
pub(super) fn database_args(connections: &Path) -> Vec<String> {
    vec![
        "--connections".to_owned(),
        connections.to_str().unwrap().to_owned(),
        "--profile".to_owned(),
        "local".to_owned(),
        // The one-release alias: `--mode bypass` would be refused unless the
        // user config allows bypass, and a piped REPL states consent here.
        "--approval-mode".to_owned(),
        "bypass".to_owned(),
    ]
}

pub(super) fn as_args(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).collect()
}

/// A profile whose SQLite file is not there: the run opens without a database
/// and the session says so once, in its own words. Returns that connections
/// file.
pub(super) fn stale_profile(dir: &Path) -> PathBuf {
    let connections = dir.join("stale-connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            dir.join("gone.sqlite").display()
        ),
    )
    .unwrap();
    connections
}

/// The turn's first database use: the call that arms `[limits.database]`.
pub(super) fn database_call() -> ScriptStep {
    ScriptStep::ToolCall {
        name: "schema_discovery".into(),
        arguments: serde_json::json!({}),
    }
}

/// Every tool result the model was handed, read off the request bodies the mock
/// recorded: the loop replays each result as a `tool` message in the next
/// request, so a result that never came back is absent here.
pub(super) fn tool_results(server: &harness::MockServer) -> Vec<String> {
    let bodies = server.bodies.lock().unwrap();
    let mut results = Vec::new();
    for body in bodies.iter() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
            continue;
        };
        for message in value["messages"].as_array().into_iter().flatten() {
            if message["role"] != "tool" {
                continue;
            }
            if let Some(content) = message["content"].as_str() {
                results.push(content.to_owned());
            }
        }
    }
    results
}

pub(super) fn stdout_of(child: &harness::SpawnedChild) -> String {
    String::from_utf8_lossy(&child.stdout_buf.lock().unwrap()).into_owned()
}

pub(super) fn stderr_of(child: &harness::SpawnedChild) -> String {
    String::from_utf8_lossy(&child.stderr_buf.lock().unwrap()).into_owned()
}
