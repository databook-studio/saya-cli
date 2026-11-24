//! The settled-question row: a Completed natural answer ends the question, so
//! the next line begins a fresh one. It is a `[limits.session]`-era scenario kept
//! as the row it was — it states no database, so with the on-use
//! `[limits.database]` budget nothing arms and the row states only that neither
//! turn is stopped. Kept beside the re-specified rows so the boundary between
//! "nothing carried" and "a fresh arm" stays visible.

use super::harness::{
    ScriptStep, spawn_child, spawn_script_server, wait_for_stream_count, wait_for_stream_timeout,
};

/// Row C: a Completed natural answer settles the question (Idle), so the
/// next line begins a FRESH scope with the full ceiling. Each turn holds
/// the source 12s INSIDE the answering segment; 12s + 12s = 24s > 20s,
/// so a carried-across budget would stop the second turn mid-stream. The
/// scaled values leave ~8s of setup overhead per turn before the first
/// turn's own delay could exhaust the ceiling on a slow CI runner.
#[cfg(unix)]
#[test]
fn a_natural_answer_ends_the_question_and_the_next_line_gets_a_fresh_budget() {
    use std::{
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![
            ScriptStep::DelayedNatural {
                delay: Duration::from_millis(12_000),
            },
            ScriptStep::DelayedNatural {
                delay: Duration::from_millis(12_000),
            },
        ],
        release,
    );

    let mut spawned = spawn_child(&server.address, 20, "budget-fresh");
    spawned.send(b"first question\n");
    wait_for_stream_timeout(
        &spawned.stdout_buf,
        b"the-final-answer",
        "first natural answer",
        Duration::from_secs(40),
    );
    // Let the completed turn settle to Idle before the next line. A bounded
    // wait (not an event: the headless stream prints no idle marker) with
    // room for a slow runner's post-turn work.
    thread::sleep(Duration::from_secs(5));
    spawned.send(b"second unrelated question\n");
    wait_for_stream_count(
        &spawned.stdout_buf,
        b"the-final-answer",
        "second natural answer",
        2,
        Duration::from_secs(40),
    );

    thread::sleep(Duration::from_secs(1));
    let stdout_text = String::from_utf8_lossy(&spawned.stdout_buf.lock().unwrap()).into_owned();
    assert!(
        !stdout_text.contains("(cancelled)"),
        "second turn was budget-stopped: {stdout_text:?}"
    );
    let stderr_text = String::from_utf8_lossy(&spawned.stderr_buf.lock().unwrap()).into_owned();
    assert!(
        !stderr_text.contains("request cancelled"),
        "budget stopped a turn: {stderr_text}"
    );

    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(count, 2, "mock should see exactly two question turns");
}
