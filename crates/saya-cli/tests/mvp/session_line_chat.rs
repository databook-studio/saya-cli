//! The plain REPL's turns run on the new chat session.
//!
//! Three facts pin the swap, each read off a real REPL child against the
//! scripted provider:
//!
//! 1. a piped message answers, a job exists for the session afterwards, and the
//!    saved session record links to that job — the run the message ran on;
//! 2. a database budget exhausted mid-turn says the shared partial sentence on
//!    stderr, in today's words;
//! 3. a clarification prints the `⏸ question` block, and the next line answers
//!    it on the same run.
//!
//! The budget of (2) is `[limits.database]`: the database budget arms on the
//! turn's first database use, so the scenario's own `schema_discovery` call is
//! what arms it before the held request runs out of time.

#[cfg(unix)]
use std::{
    net::TcpListener,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

#[cfg(unix)]
use crate::session_question_budget::harness::{
    self, ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream,
    wait_for_stream_timeout,
};

#[cfg(unix)]
const ANSWER: &str = "the-final-answer";

#[cfg(unix)]
fn listener() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

/// The one config every row states: sharing is the operator's consent (the mock
/// is not a local endpoint), memory is off so no extraction adds a request, and
/// the scenario states its own database budget.
#[cfg(unix)]
fn config(database_elapsed_seconds: u64) -> String {
    format!(
        "[ai]\nallow_data_sharing = true\n\n\
         [limits.database]\nelapsed_seconds = {database_elapsed_seconds}\n\n\
         [memory]\nmode = 'off'\n"
    )
}

/// The saved session record this child wrote, parsed: the store's file under
/// the child's own session root. Waits for the per-turn save, which follows the
/// turn the record is about.
#[cfg(unix)]
fn saved_record(session_root: &std::path::Path) -> serde_json::Value {
    use std::{thread, time::Instant};
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let mut files: Vec<_> = std::fs::read_dir(session_root)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|kind| kind == "json"))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        if let Some(path) = files.last() {
            let bytes = std::fs::read(path).expect("the record reads");
            let record: serde_json::Value =
                serde_json::from_slice(&bytes).expect("the record is JSON");
            if record.get("run").is_some() {
                return record;
            }
        }
        assert!(
            Instant::now() < deadline,
            "the child saved a session record under {session_root:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// The job directories a child left under its own coding state root.
#[cfg(unix)]
fn job_dirs(config_root: &std::path::Path) -> Vec<String> {
    let coding = config_root.join("coding");
    let Ok(entries) = std::fs::read_dir(&coding) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        // A job's directory is the one holding its own request: the coding root
        // also holds the run-wide snapshots, worktrees and lease folders.
        .filter(|entry| entry.path().join("request.json").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Every file and directory under `root`, for an assertion's message.
#[cfg(unix)]
fn tree(root: &std::path::Path) -> String {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            out.push(path.display().to_string());
            if path.is_dir() {
                walk(&path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.join(", ")
}

#[cfg(unix)]
#[test]
fn a_piped_message_answers_on_a_job_the_saved_record_links() {
    let mut server = spawn_script_server(
        listener(),
        vec![ScriptStep::Summary(ANSWER.to_owned())],
        Arc::new(AtomicBool::new(false)),
    );
    let mut child = spawn_child_with_config_extra(&server.address, &config(60), "line-link", &[]);
    let config_root = child.config_root().to_path_buf();
    let session_root = child.session_root().to_path_buf();
    child.send(b"how much revenue is there\n");
    wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    // The per-turn save follows the turn, so the record and the job are read
    // before the child's roots are removed by `finish`.
    let record = saved_record(&session_root);
    let jobs = job_dirs(&config_root);
    let stdout = child.stdout_buf.clone();
    child.send(b"/exit\n");
    child.finish();
    server.join();

    let stdout = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    assert!(
        stdout.contains(ANSWER),
        "the message was answered through the session: {stdout}"
    );
    assert_eq!(
        jobs.len(),
        1,
        "the message opened one job for the session: {jobs:?} under {}",
        tree(&config_root)
    );
    let linked = record
        .get("run")
        .and_then(|run| run.get("job"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(
        linked,
        Some(jobs[0].as_str()),
        "the saved record links to the run it ran on: {record}"
    );
}

#[cfg(unix)]
#[test]
fn a_database_budget_stop_says_the_partial_sentence_on_stderr() {
    // The database the scenario reads: a seeded SQLite file and the profile
    // that names it, written under the child's own config root before it
    // starts.
    let root = harness::config_root_for("line-budget");
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("golden.sqlite");
    crate::common::sqlite_fixture(
        &database,
        &[
            "CREATE TABLE revenue (amount INTEGER);",
            "INSERT INTO revenue (amount) VALUES (10), (20);",
        ],
    );
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let mut server = spawn_script_server(
        listener(),
        vec![
            ScriptStep::ToolCall {
                name: "schema_discovery".into(),
                arguments: serde_json::json!({}),
            },
            // The next request is held in the source for longer than the
            // ceiling the call above armed: the turn's own deadline is what
            // ends it, not a reply.
            ScriptStep::DelayedNatural {
                delay: Duration::from_secs(15),
            },
            ScriptStep::Summary(ANSWER.to_owned()),
        ],
        Arc::new(AtomicBool::new(false)),
    );
    let mut child = spawn_child_with_config_extra(
        &server.address,
        &config(8),
        "line-budget",
        &[
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            // The database tools are approved per call; a piped REPL cannot
            // answer a prompt, so the scenario states the consent itself.
            "--approval-mode",
            "bypass",
        ],
    );
    child.send(b"how much revenue is there\n");
    // The call has to reach the plugin — that is what arms the budget — before
    // the held request can run out of it.
    wait_for_stream_timeout(
        &child.stdout_buf,
        b"schema_discovery",
        "the plugin-served call",
        Duration::from_secs(20),
    );
    wait_for_stream_timeout(
        &child.stderr_buf,
        b"the run stopped on its elapsed-time budget; any answer it produced is partial or absent",
        "the partial sentence",
        Duration::from_secs(40),
    );
    let stderr = child.stderr_buf.clone();
    child.send(b"/exit\n");
    child.finish();
    server.join();
    let stderr = String::from_utf8_lossy(&stderr.lock().unwrap()).into_owned();
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

#[cfg(unix)]
#[test]
fn a_clarification_prints_the_question_block_and_the_next_line_answers_it() {
    let mut server = spawn_script_server(
        listener(),
        vec![
            ScriptStep::Clarify {
                delay: Duration::from_secs(0),
            },
            ScriptStep::Summary(ANSWER.to_owned()),
        ],
        Arc::new(AtomicBool::new(false)),
    );
    let root = harness::config_root_for("line-clarify");
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("golden.sqlite");
    crate::common::sqlite_fixture(
        &database,
        &[
            "CREATE TABLE revenue (amount INTEGER);",
            "INSERT INTO revenue (amount) VALUES (10), (20);",
        ],
    );
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let mut child = spawn_child_with_config_extra(
        &server.address,
        &config(60),
        "line-clarify",
        &[
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
            "--approval-mode",
            "bypass",
        ],
    );
    child.send(b"which rows do you mean\n");
    wait_for_stream(
        &child.stdout_buf,
        "⏸ question".as_bytes(),
        "the pending question",
    );
    let stdout = child.stdout_buf.clone();
    let block = {
        let text = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
        text.lines()
            .skip_while(|line| !line.starts_with("⏸ question"))
            .take(4)
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        block.contains("Which one?"),
        "the model's own question is on the block: {block}"
    );
    assert!(
        block.contains("1. a") && block.contains("2. b"),
        "the options are numbered under it: {block}"
    );
    child.send(b"option a\n");
    wait_for_stream(&child.stdout_buf, ANSWER.as_bytes(), "the answer");
    child.send(b"/exit\n");
    child.finish();
    server.join();
}
