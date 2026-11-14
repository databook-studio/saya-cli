/// `/clear` re-arms the latched trigger in the plain REPL: after the latched
/// failure, `/clear` empties the working memory (and the latch with it), and
/// four fresh turns whose fourth crosses the threshold produce a NEW
/// summariser request and the automatic success line.
#[cfg(unix)]
#[test]
fn clear_re_arms_the_automatic_trigger_headless() {
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
            ScriptStep::Summary("a summary of the cleared conversation".into()),
        ],
        release,
    );

    let mut spawned = spawn_child_with_config(&server.address, &auto_config(), "auto-clear");
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

    spawned.send(b"/clear\n");
    wait_for_stream_timeout(
        &spawned.stdout_buf,
        b"Conversation context cleared",
        "the /clear confirmation line",
        Duration::from_secs(15),
    );
    for line in [
        "fresh first question\n",
        "fresh second question\n",
        "fresh third question\n",
        "fresh fourth question\n",
    ] {
        spawned.send(line.as_bytes());
    }
    wait_for_stream_count(
        &spawned.stdout_buf,
        b"the-final-answer",
        "the eight natural answers",
        8,
        Duration::from_secs(20),
    );
    wait_for_stream_timeout(
        &spawned.stdout_buf,
        b"Automatic compaction: Compacted",
        "the automatic compaction success line after /clear",
        Duration::from_secs(15),
    );

    let stdout_buf = spawned.stdout_buf.clone();
    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(
        count, 10,
        "4 answers + 1 failed summariser + 4 fresh answers + 1 re-armed summariser"
    );
    let bodies = server.bodies.lock().unwrap();
    assert_eq!(
        summariser_request_count(&bodies),
        2,
        "one failed attempt before /clear and one re-armed success after: {bodies:?}"
    );
    let stdout_text = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).into_owned();
    assert_eq!(
        stdout_text
            .matches("Automatic compaction: Compacted")
            .count(),
        1,
        "exactly one automatic success line, after /clear: {stdout_text:?}"
    );
}
