//! A question-budget stop in the plain REPL must say the same
//! truthful partial wording `ask` says, not "Request cancelled." or the
//! provider error.
//!
//! The budget is `[limits.database]` and it arms on the turn's first database
//! use (`ref/05` §2-§4.1), so this row states a database and lets the model
//! make one database call before the held request: that call is what arms the
//! elapsed ceiling the turn then stops on. Without a database the row would
//! never be bounded at all — which is the truth this release states, and why
//! the old no-database scenario could no longer stop.

#[cfg(unix)]
#[path = "session_budget_wording/harness.rs"]
pub(crate) mod harness;

/// The database budget's one row: its elapsed ceiling. Every other dimension
/// keeps the resolved default, which this slow row never reaches.
#[cfg(unix)]
fn database_config(elapsed_seconds: u64) -> String {
    format!(
        "[ai]\nallow_data_sharing = true\n\n\
         [limits.database]\nelapsed_seconds = {elapsed_seconds}\n"
    )
}

#[cfg(unix)]
#[test]
fn a_question_budget_stop_says_the_partial_wording_not_request_cancelled() {
    use std::{
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use harness::{
        ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream_timeout,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![
            // Step 1: the model's own database call — the turn's first database
            // use, and therefore what arms `[limits.database]`.
            ScriptStep::ToolCall {
                name: "schema_discovery".into(),
                arguments: serde_json::json!({}),
            },
            // Step 2: the request that carries the tool's result is held open
            // and silent, so the armed 2s elapsed ceiling is what ends the turn.
            ScriptStep::HoldOpen {
                marker: String::new(),
            },
        ],
        release.clone(),
    );
    let (root, connections) = crate::common::database_fixture("budget-wording");
    let mut spawned = spawn_child_with_config_extra(
        &server.address,
        &database_config(2),
        "budget-wording",
        &[
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            // The database tools require approval; a piped REPL cannot answer a
            // prompt, so the scenario states the consent itself.
            "--approval-mode",
            "bypass",
        ],
    );
    spawned.send(b"any question at all\n");

    // The silent-held establishment never answers, so the 2s database budget is
    // what ends the turn. The answering recovery first prints its (cancelled)
    // line on stdout; the settled turn must then report the budget truthfully on
    // stderr.
    wait_for_stream_timeout(
        &spawned.stdout_buf,
        b"(cancelled)",
        "mid-turn budget stop",
        Duration::from_secs(15),
    );
    wait_for_stream_timeout(
        &spawned.stderr_buf,
        b"the run stopped on its elapsed-time budget; any answer it produced is partial or absent",
        "truthful budget wording",
        Duration::from_secs(15),
    );
    release.store(true, Ordering::SeqCst);

    let stdout_text = String::from_utf8_lossy(&spawned.stdout_buf.lock().unwrap()).into_owned();
    let stderr_text = String::from_utf8_lossy(&spawned.stderr_buf.lock().unwrap()).into_owned();
    assert!(
        !stderr_text.contains("Request cancelled."),
        "budget stop must not say Request cancelled.: {stderr_text}"
    );
    assert!(
        !stdout_text.contains("Request cancelled."),
        "budget stop must not say Request cancelled. on stdout: {stdout_text:?}"
    );

    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(
        count, 2,
        "mock should see the arming call and the held request"
    );
    let _ = std::fs::remove_dir_all(root);
}
