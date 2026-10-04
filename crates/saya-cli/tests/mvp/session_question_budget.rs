#[cfg(unix)]
#[path = "session_question_budget/harness.rs"]
mod harness;

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

    use harness::{ScriptStep, spawn_child, spawn_script_server, wait_for_stream};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![
            ScriptStep::Clarify { delay: Duration::from_secs(0) },
            ScriptStep::Natural,
        ],
        release,
    );

    let mut spawned = spawn_child(&server.address, 2, "budget");
    spawned.send(b"which one do you mean\n");

    wait_for_stream(&spawned.stdout_buf, "⏸ question".as_bytes(), "pending clarification");
    // The question is parked awaiting the human reply, so the 3s idle wait
    // here must not count against the 2s question elapsed budget.
    thread::sleep(Duration::from_secs(3));
    spawned.send(b"option a\n");

    wait_for_stream(&spawned.stdout_buf, b"the-final-answer", "final answer");
    thread::sleep(Duration::from_secs(2));
    let stderr_text = String::from_utf8_lossy(&spawned.stderr_buf.lock().unwrap()).into_owned();
    assert!(
        !stderr_text.contains("request cancelled"),
        "budget stopped the reply turn: {stderr_text}"
    );

    spawned.send(b"/exit\n");

    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert!(count >= 2, "mock should see question + reply turns");
}
