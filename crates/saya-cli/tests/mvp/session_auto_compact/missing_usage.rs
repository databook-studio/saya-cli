/// The last answering answer carries NO usage frame: the numerator is
/// unknown, and unknown never fires — even at a turn count and a reported
/// history that would otherwise cross the threshold.
#[cfg(unix)]
#[test]
fn unreported_usage_never_triggers_compaction() {
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
            ScriptStep::NaturalNoUsage,
        ],
        release,
    );

    let mut spawned = spawn_child_with_config(&server.address, &auto_config(), "auto-unknown");
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
    assert_eq!(count, 4, "no summariser request may fire on unknown usage");
    let stdout_text = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).into_owned();
    assert!(
        !stdout_text.contains("Automatic compaction"),
        "an automatic line appeared on unknown usage: {stdout_text:?}"
    );
}

/// Counts the recorded bodies that are summariser requests: the compaction
/// prompt's instruction text is unique to them (session_compact.rs), and a
/// failed attempt may be retried, so the count — never the position — is the
/// oracle.
#[cfg(unix)]
fn summariser_request_count(bodies: &[String]) -> usize {
    bodies
        .iter()
        .filter(|body| body.contains("Summarise the following earlier conversation turns"))
        .count()
}
