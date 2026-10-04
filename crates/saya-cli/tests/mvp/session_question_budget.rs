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
            ScriptStep::Clarify {
                delay: Duration::from_secs(0),
            },
            ScriptStep::Natural,
        ],
        release,
    );

    let mut spawned = spawn_child(&server.address, 2, "budget");
    spawned.send(b"which one do you mean\n");

    wait_for_stream(
        &spawned.stdout_buf,
        "⏸ question".as_bytes(),
        "pending clarification",
    );
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

/// Row A: the 2.5s source-held delay of turn 1 is ACTIVE elapsed time, so
/// the reply turn re-begins the SAME scope with only ~1.5s of the 4s
/// ceiling left. A reset that minted a fresh 4s grant on the reply would
/// stop at ~4s instead, which the `elapsed < 3.0s` oracle catches; the
/// `> 0.5s` bound only guards against an instant (non-budget) stop.
#[cfg(unix)]
#[test]
fn a_clarification_reply_runs_on_the_remaining_budget_not_a_fresh_grant() {
    use std::{
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };

    use harness::{ScriptStep, spawn_child, spawn_script_server, wait_for_stream_timeout};

    // Script: turn 1 holds the source 2.5s INSIDE the answering segment
    // (a real observed delay before writing anything), then asks; the
    // 4s ceiling therefore has ~1.5s left when the reply turn begins.
    // Turn 2's request is accepted and held SILENT — the establishment
    // (60s) and per-chunk idle (90s) timeouts never fire, so the runtime's
    // own remaining-elapsed timer is what ends the reply turn while the
    // socket is still open. Requests past slot 1 get a harmless
    // completion (only reached if something retries after the stop).
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![
            ScriptStep::Clarify {
                delay: Duration::from_millis(2500),
            },
            ScriptStep::HoldOpen {
                marker: String::new(),
            },
        ],
        release.clone(),
    );

    let mut spawned = spawn_child(&server.address, 4, "budget-remain");
    spawned.send(b"which one do you mean\n");

    wait_for_stream_timeout(
        &spawned.stdout_buf,
        "\u{23f8} question".as_bytes(),
        "pending clarification",
        Duration::from_secs(10),
    );
    let t_reply = Instant::now();
    spawned.send(b"option a\n");

    // The reply turn must still be RUNNING when the remaining budget fires
    // — but the headless stream only proves "running" through a visible
    // event, and a silent-held socket emits none. So the mid-turn oracle is
    // the provider-recovery line the Cancelled retry prints on STDOUT
    // (`(cancelled)` names the answering token's stop, not a transport
    // error): it lands when the remaining budget fires mid-establishment.
    // A FRESH 4s grant on the reply would land it at ~4s instead of ~1.5s,
    // which the `< 3.0s` oracle catches; `> 0.5s` guards an instant stop.
    // (Keep the wait generous: the stop lands ~1.5s after the reply, but
    // CI scheduling adds jitter.)
    let cancelled_seen = wait_for_stdout_timeout(
        &spawned.stdout_buf,
        b"(cancelled)",
        "mid-turn budget stop",
        Duration::from_secs(15),
    );
    let elapsed = t_reply.elapsed();
    release.store(true, Ordering::SeqCst);

    assert!(
        elapsed < Duration::from_secs_f64(3.0),
        "reply turn ran {elapsed:?}: a fresh grant would stop at ~4s"
    );
    assert!(
        elapsed > Duration::from_millis(500),
        "reply turn stopped instantly ({elapsed:?}): not a budget stop"
    );
    assert!(
        cancelled_seen,
        "no mid-turn (cancelled) stop line on stdout within 15s of the reply"
    );

    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(count, 2, "mock should see exactly question + reply turns");
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
    // into a boolean so the elapsed oracle (not the wait) decides red.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_stream_timeout(buffer, needle, label, timeout);
    }));
    result.is_ok()
}
