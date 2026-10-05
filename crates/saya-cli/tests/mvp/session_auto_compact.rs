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
