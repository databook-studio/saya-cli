//! Real-binary REPL rows for the automatic-compaction turn boundary: a
//! completed turn whose answering input sits at or above the compact
//! threshold fires EXACTLY ONE summariser request through the same
//! operation manual `/compact` runs; a turn below the threshold and a turn
//! with an unreported numerator never fire.

#[cfg(unix)]
use crate::session_question_budget::harness;

#[cfg(unix)]
const WINDOW: u64 = 1_000_000;

#[cfg(unix)]
fn auto_config() -> String {
    format!(
        "[ai]\ninvestigation_max_elapsed_seconds = 30\ncontext_window_tokens = {WINDOW}\ncompaction = \"auto\"\n"
    )
}

/// A turn at 96% of the window crosses the 95% threshold: the mock must see
/// the four answering requests plus exactly one summariser request whose
/// body is the compaction prompt carrying the earlier conversation, and the
/// automatic success line must reach the transcript.
#[cfg(unix)]
#[test]
fn a_turn_over_the_threshold_triggers_one_automatic_compaction() {
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
            ScriptStep::Summary("short summary of the earlier conversation".into()),
        ],
        release,
    );

    let mut spawned = spawn_child_with_config(&server.address, &auto_config(), "auto-compact");
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
    wait_for_stream_timeout(
        &spawned.stdout_buf,
        b"Automatic compaction: Compacted",
        "the automatic compaction success line",
        Duration::from_secs(15),
    );

    let stdout_buf = spawned.stdout_buf.clone();
    spawned.send(b"/exit\n");
    spawned.finish();
    server.join();
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(count, 5, "mock should see 4 answers + exactly 1 summariser");
    let bodies = server.bodies.lock().unwrap();
    let summariser = &bodies[4];
    assert!(
        summariser.contains("Summarise the following earlier conversation turns"),
        "the 5th request is not the summariser prompt: {summariser:?}"
    );
    assert!(
        summariser.contains("first question"),
        "the summariser prompt lost the earlier conversation: {summariser:?}"
    );
    let stdout_text = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).into_owned();
    assert!(
        stdout_text.contains("Compacted 2 turns"),
        "the success line should name the compacted turns: {stdout_text:?}"
    );
}

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
