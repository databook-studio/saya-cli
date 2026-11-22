//! The database budget and a clarification's reply: a reply after a long idle
//! wait is not stopped, and the reply turn arms its own ceiling at its first
//! database use.
//!
//! Both rows are `[limits.database]` rows (`ref/05` §2-§4.1): the budget arms on
//! a turn's first database use, so each row states a database and scripts the
//! database call that arms the ceiling it is about. A clarification is the
//! database plugin's own tool, so a row that asks one needs a profile anyway.
//!
//! The retired `[limits.session]` question scope bounded every turn of a
//! conversation from the question's first line. Nothing carries across a
//! clarification any more, which is what the second row states.

#[cfg(unix)]
#[path = "session_question_budget/harness.rs"]
pub(crate) mod harness;

/// The settled-question row lives in its own file: the re-specified rows above
/// are the ones this job changed, and the row here is kept as it was.
#[cfg(unix)]
#[path = "session_question_budget/turn_scope.rs"]
mod turn_scope;

/// The database budget's one row: its elapsed ceiling. Every other dimension
/// keeps the resolved default.
#[cfg(unix)]
fn database_config(elapsed_seconds: u64) -> String {
    format!(
        "[ai]\nallow_data_sharing = true\n\n\
         [limits.database]\nelapsed_seconds = {elapsed_seconds}\n"
    )
}

/// The arguments a row that connects to its fixture passes: the profile, and
/// `bypass` because a piped REPL cannot answer the database tools' approval
/// prompt itself.
#[cfg(unix)]
fn database_args(connections: &std::path::Path) -> Vec<String> {
    vec![
        "--connections".to_owned(),
        connections.to_str().unwrap().to_owned(),
        "--profile".to_owned(),
        "local".to_owned(),
        "--approval-mode".to_owned(),
        "bypass".to_owned(),
    ]
}

/// One scripted database call: the turn's first database use, which is what arms
/// `[limits.database]` for that turn.
#[cfg(unix)]
fn database_call() -> harness::ScriptStep {
    harness::ScriptStep::ToolCall {
        name: "schema_discovery".into(),
        arguments: serde_json::json!({}),
    }
}

#[cfg(unix)]
#[test]
fn a_clarification_reply_after_a_long_idle_wait_is_not_budget_stopped() {
    use std::{
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };

    use harness::{
        ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![
            // Turn 1: arm the budget, then ask — the ask ends the turn.
            database_call(),
            ScriptStep::Clarify {
                delay: Duration::from_secs(0),
            },
            // Turn 2, the reply: arm this turn's budget at its own first use,
            // then answer.
            database_call(),
            ScriptStep::Natural,
        ],
        release,
    );

    let (root, connections) = crate::common::database_fixture("question-idle");
    let args = database_args(&connections);
    let mut spawned = spawn_child_with_config_extra(
        &server.address,
        &database_config(20),
        "budget",
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    spawned.send(b"which one do you mean\n");

    wait_for_stream(
        &spawned.stdout_buf,
        "⏸ question".as_bytes(),
        "pending clarification",
    );
    // The question is parked awaiting the human reply, so the idle wait here
    // must not count against any budget. The idle must EXCEED the ceiling for
    // that assertion to have teeth: were the parked idle charged to the reply's
    // ceiling, the reply turn would find it exhausted and stop instantly.
    thread::sleep(Duration::from_secs(23));
    spawned.send(b"option a\n");

    wait_for_stream(&spawned.stdout_buf, b"the-final-answer", "final answer");
    thread::sleep(Duration::from_secs(2));
    let stderr_text = String::from_utf8_lossy(&spawned.stderr_buf.lock().unwrap()).into_owned();
    assert!(
        !stderr_text.contains("request cancelled"),
        "budget stopped the reply turn: {stderr_text}"
    );
    assert!(
        !stderr_text.contains("the run stopped on its"),
        "the reply turn was budget-stopped after a parked idle wait: {stderr_text}"
    );

    spawned.send(b"/exit\n");

    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert!(
        count >= 2,
        "mock should see question + reply turns: {count}"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// The reply turn's own ceiling is armed at its own first database use, so the
/// time the question's turn spent is not carried into it. Turn 1 holds its own
/// source 12s inside a database-armed turn, so a reply that inherited the
/// question's 20s ceiling would have ~8s left and stop before 14s; a reply that
/// arms its own ceiling runs past it. The `> 14s` oracle catches the carried
/// budget and the `< 30s` bound catches a turn nothing bounded at all.
#[cfg(unix)]
#[test]
fn a_clarification_reply_arms_its_own_database_budget_at_its_first_use() {
    use std::{
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };

    use harness::{
        ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream_timeout,
    };

    // Turn 1: arm, then hold the source 12s inside the answering segment before
    // asking. Turn 2, the reply: arm this turn's ceiling with its own database
    // call, then hold the next request silent — the armed 20s is what ends the
    // reply turn, and it starts at the arm.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![
            database_call(),
            ScriptStep::Clarify {
                delay: Duration::from_millis(12_000),
            },
            database_call(),
            ScriptStep::HoldOpen {
                marker: String::new(),
            },
        ],
        release.clone(),
    );

    let (root, connections) = crate::common::database_fixture("question-remain");
    let args = database_args(&connections);
    let mut spawned = spawn_child_with_config_extra(
        &server.address,
        &database_config(20),
        "budget-remain",
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    spawned.send(b"which one do you mean\n");

    wait_for_stream_timeout(
        &spawned.stdout_buf,
        "⏸ question".as_bytes(),
        "pending clarification",
        Duration::from_secs(60),
    );
    let t_reply = Instant::now();
    spawned.send(b"option a\n");

    // The reply turn must still be RUNNING when its own ceiling fires — but the
    // headless stream only proves "running" through a visible event, and a
    // silent-held socket emits none. So the mid-turn oracle is the
    // provider-recovery line the Cancelled retry prints on STDOUT
    // (`(cancelled)` names the answering token's stop, not a transport error):
    // it lands when the reply turn's armed ceiling fires.
    let cancelled_seen = wait_for_stdout_timeout(
        &spawned.stdout_buf,
        b"(cancelled)",
        "mid-turn budget stop",
        Duration::from_secs(40),
    );
    let elapsed = t_reply.elapsed();
    release.store(true, Ordering::SeqCst);

    assert!(
        elapsed > Duration::from_secs_f64(14.0),
        "reply turn ran {elapsed:?}: the question's leftover ceiling would have stopped it"
    );
    assert!(
        elapsed < Duration::from_secs_f64(30.0),
        "reply turn ran {elapsed:?}: its own ceiling is 20s"
    );
    assert!(
        cancelled_seen,
        "no mid-turn (cancelled) stop line on stdout within 40s of the reply"
    );

    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(count, 4, "the arming call and the held request, twice");
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
fn wait_for_stdout_timeout(
    buffer: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    needle: &[u8],
    label: &str,
    timeout: std::time::Duration,
) -> bool {
    use harness::wait_for_stream_timeout;
    // Reuse the harness waiter for its failure dump; convert the panic
    // into a boolean so the elapsed oracle (not the wait) decides the result.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_stream_timeout(buffer, needle, label, timeout);
    }));
    result.is_ok()
}
