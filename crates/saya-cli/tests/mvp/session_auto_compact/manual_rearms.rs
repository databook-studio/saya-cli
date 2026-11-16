/// A successful manual `/compact` re-arms the automatic trigger: after the
/// latched failure of the previous row's setup, `/compact` runs the same
/// summariser with a reply that preserves the pinned token and succeeds; then
/// one more eligible answer (98%) makes the mock receive a NEW summariser
/// request — the oracle a latched trigger could never produce. The re-armed
/// firing's success line is the only "Automatic compaction: Compacted" line.
#[cfg(unix)]
#[test]
fn a_manual_compact_success_re_arms_the_automatic_trigger() {
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
            ScriptStep::Summary("kept select count from orders and earlier work".into()),
            ScriptStep::NaturalWithUsage {
                input_tokens: 980_000,
            },
            ScriptStep::Summary("the second summary of the earlier conversation".into()),
        ],
        release,
    );

    let mut spawned = spawn_child_with_config(&server.address, &auto_config(), "auto-rearm");
    for line in [
        "select count from orders\n",
        "second question\n",
        "third question\n",
        "fourth question\n",
        "fifth question\n",
    ] {
        spawned.send(line.as_bytes());
    }
    wait_for_stream_count(
        &spawned.stdout_buf,
        b"the-final-answer",
        "the five natural answers",
        5,
        Duration::from_secs(20),
    );
    wait_for_stream_timeout(
        &spawned.stderr_buf,
        b"It will not retry on its own",
        "the automatic compaction failure line",
        Duration::from_secs(15),
    );

    spawned.send(b"/compact\n");
    wait_for_stream_timeout(
        &spawned.stdout_buf,
        b"Compacted 3 turns",
        "the manual compaction success line",
        Duration::from_secs(15),
    );

    spawned.send(b"sixth question\n");
    wait_for_stream_count(
        &spawned.stdout_buf,
        b"the-final-answer",
        "the six natural answers",
        6,
        Duration::from_secs(20),
    );
    wait_for_stream_timeout(
        &spawned.stdout_buf,
        b"Automatic compaction: Compacted",
        "the re-armed automatic compaction success line",
        Duration::from_secs(15),
    );

    let stdout_buf = spawned.stdout_buf.clone();
    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(
        count, 9,
        "4 answers + 1 failed summariser + turn 5 + manual + turn 6 + re-armed summariser"
    );
    let bodies = server.bodies.lock().unwrap();
    assert_eq!(
        summariser_request_count(&bodies),
        3,
        "one failed automatic + one manual + one re-armed automatic summariser: {bodies:?}"
    );
    assert!(
        bodies.last().is_some_and(|body| {
            body.contains("Summarise the following earlier conversation turns")
        }),
        "the last request must be the re-armed trigger's summariser: {bodies:?}"
    );
    let stdout_text = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).into_owned();
    assert_eq!(
        stdout_text
            .matches("Automatic compaction: Compacted")
            .count(),
        1,
        "exactly one automatic success line, from the re-armed trigger: {stdout_text:?}"
    );
}
