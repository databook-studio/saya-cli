//! PH-H1: a question-budget stop in the plain REPL must say the same
//! truthful partial wording `ask` says, not "Request cancelled." or the
//! provider error.

#[cfg(unix)]
#[path = "session_budget_wording/harness.rs"]
pub(crate) mod harness;

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

    use harness::{ScriptStep, spawn_child, spawn_script_server, wait_for_stream_timeout};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![ScriptStep::HoldOpen {
            marker: String::new(),
        }],
        release.clone(),
    );

    let mut spawned = spawn_child(&server.address, 2, "budget-wording");
    spawned.send(b"any question at all\n");

    // The silent-held establishment never answers, so the 2s question
    // budget is what ends the turn. The answering recovery first prints
    // its (cancelled) line on stdout; the settled turn must then report
    // the budget truthfully on stderr.
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
    assert_eq!(count, 1, "mock should see only the question turn");
}
