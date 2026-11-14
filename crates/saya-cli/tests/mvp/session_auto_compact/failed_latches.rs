/// A failed automatic compaction latches the trigger: turn 4 crosses the
/// threshold and the summariser's reply drops the pinned token from turn 1's
/// question, so `validate` rejects it (one attempt — a rejected summary is
/// never retried) and the failure line names the no-retry policy. Turn 5 is
/// still eligible by occupancy (97%) but the latch holds: the mock sees only
/// turn 5's answering request, and no second automatic line appears.
#[cfg(unix)]
#[test]
fn a_failed_automatic_compaction_latches_and_does_not_retry_on_a_later_eligible_answer() {
    use std::{
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use harness::{
        ScriptStep, spawn_child_with_config, spawn_script_server, wait_for_stream_count,
        wait_for_stream_timeout,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![
            ScriptStep::NaturalWithUsage {
                input_tokens: 100_000,
            },
            ScriptStep::NaturalWithUsage {
                input_tokens: 200_000,
            },
            ScriptStep::NaturalWithUsage {
                input_tokens: 300_000,
            },
            ScriptStep::NaturalWithUsage {
                input_tokens: 960_000,
            },
            ScriptStep::Summary("a summary that drops the pinned anchor".into()),
            ScriptStep::NaturalWithUsage {
                input_tokens: 970_000,
            },
        ],
        release,
    );

    let mut spawned = spawn_child_with_config(&server.address, &auto_config(), "auto-latch");
    for line in [
        "select count from orders\n",
        "second question\n",
        "third question\n",
        "fourth question\n",
    ] {
        spawned.send(line.as_bytes());
    }
    wait_for_stream_count(
        &spawned.stdout_buf,
        b"the-final-answer",
        "the four natural answers",
        4,
        Duration::from_secs(20),
    );
    wait_for_stream_timeout(
        &spawned.stderr_buf,
        b"It will not retry on its own",
        "the automatic compaction failure line",
        Duration::from_secs(15),
    );

    spawned.send(b"fifth question\n");
    wait_for_stream_count(
        &spawned.stdout_buf,
        b"the-final-answer",
        "the five natural answers",
        5,
        Duration::from_secs(20),
    );
    std::thread::sleep(Duration::from_millis(750));

    let stdout_buf = spawned.stdout_buf.clone();
    let stderr_buf = spawned.stderr_buf.clone();
    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(
        count, 6,
        "turn 5 adds only its answering request: 4 answers + 1 failed summariser + 1 answer"
    );
    let bodies = server.bodies.lock().unwrap();
    assert_eq!(
        summariser_request_count(&bodies),
        1,
        "the latched trigger must not send a new summariser on turn 5: {bodies:?}"
    );
    let stdout_text = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).into_owned();
    assert!(
        !stdout_text.contains("Automatic compaction: Compacted"),
        "no automatic success may appear after a latched failure: {stdout_text:?}"
    );
    let stderr_text = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).into_owned();
    assert_eq!(
        stderr_text.matches("It will not retry on its own").count(),
        1,
        "exactly one automatic failure line, from turn 4 only: {stderr_text:?}"
    );
}
