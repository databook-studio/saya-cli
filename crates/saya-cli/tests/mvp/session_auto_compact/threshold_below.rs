/// The last turn reports 40% of the window — far below the threshold: only
/// the four answering requests may reach the mock, and no automatic line
/// may appear. The settle runs before the loop reads the next line, so the
/// (absent) automatic line is decided before `/exit` is processed.
#[cfg(unix)]
#[test]
fn a_turn_below_the_threshold_triggers_no_compaction() {
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
                input_tokens: 400_000,
            },
        ],
        release,
    );

    let mut spawned = spawn_child_with_config(&server.address, &auto_config(), "auto-low");
    for line in [
        "first question\n",
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

    let stdout_buf = spawned.stdout_buf.clone();
    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(
        count, 4,
        "no summariser request may fire below the threshold"
    );
    let stdout_text = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).into_owned();
    assert!(
        !stdout_text.contains("Automatic compaction"),
        "an automatic line appeared below the threshold: {stdout_text:?}"
    );
}
