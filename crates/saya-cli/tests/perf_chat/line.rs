//! The line front's measures: how long the first message takes to stream, what
//! a steady turn costs beyond the scripted provider, how many request bytes a
//! turn sends, and how many bytes its own state writes.
//!
//! One child serves every turn of one run: the front is the same process the
//! operator keeps open, so a per-turn cost measured across a single session is
//! the cost the plan's bars are about. Every turn's answer is a unique marker
//! the scripted provider was given, so each latency is the interval between the
//! write of a line and the instant that line's answer reached the stream.

use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
    thread,
    time::Duration,
};

use crate::{
    child::PerfChild,
    dialect::Dialect,
    fixture,
    harness::{self, MockServer, ScriptStep},
    record::{self, RunRecord},
    spawn::Roots,
};

/// The provider delay every steady turn is scripted with. The harness measures
/// it by the script it wrote, and subtracts it from the turn's wall time.
pub(crate) const TURN_DELAY: Duration = Duration::from_millis(200);
/// The steady turns one run measures: the plan's p50/p95 need at least twenty.
pub(crate) const STEADY_TURNS: usize = 21;
/// The answer the first message is scripted to stream.
const FIRST_ANSWER: &str = "perf-first-event";
/// The answer the self-check is scripted to stream.
const SELF_CHECK_ANSWER: &str = "perf-self-check";
/// How long one turn may take before the harness calls it a hang.
const TURN_TIMEOUT: Duration = Duration::from_secs(60);

/// One turn's wall time and the provider delay that was scripted into it.
pub(crate) struct TurnSample {
    pub(crate) wall: Duration,
    pub(crate) delay: Duration,
}

/// The turn's cost beyond the provider: its wall time with the provider's own
/// scripted delay taken off. This function is the harness's one subtraction,
/// and its self-check fails if it is removed.
pub(crate) fn overhead(sample: &TurnSample) -> Duration {
    sample.wall.saturating_sub(sample.delay)
}

/// The marker the steady turn at `index` is answered with.
pub(crate) fn marker(index: usize) -> String {
    format!("perf-turn-{index}")
}

/// The workspace a run binds: the run's own directory under its config root,
/// never this repository — which holds the run's state root inside `target/`.
pub(crate) fn connection_workspace(roots: &Roots) -> std::path::PathBuf {
    roots.config_root.join("cwd")
}

/// A mock server over `script`, held open for as many requests as it has steps
/// and for long enough that an idle window does not outlast it before the first
/// request arrives.
pub(crate) fn server(script: Vec<ScriptStep>) -> MockServer {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let limit = script.len() + 2;
    harness::spawn_script_server_limit(
        listener,
        script,
        Arc::new(AtomicBool::new(false)),
        limit,
        Duration::from_secs(600),
    )
}

/// The idle CPU of the line front, read from the OS's accounting of the live
/// child: the front is a process waiting on its own stdin.
#[cfg(unix)]
fn idle_cpu(child: &PerfChild) -> Option<f64> {
    crate::cpu::idle_percent(child.pid(), crate::IDLE_WINDOW)
}

/// Windows has no per-process CPU accounting here, so the window is reported as
/// skipped rather than guessed at.
#[cfg(not(unix))]
fn idle_cpu(_child: &PerfChild) -> Option<f64> {
    None
}

/// Measures (a), (b), (c) and (f) on one session of `bin`, launched in the
/// posture `dialect` says `bin` names.
pub(crate) fn measure(bin: &Path, dialect: &Dialect, record: &mut RunRecord) {
    let roots = Roots::new(&format!("line-run{}", record.run));
    let connections = fixture::sqlite_profile(&roots.config_root);
    let workspace = connection_workspace(&roots);
    let mut script = vec![ScriptStep::Summary(FIRST_ANSWER.to_owned())];
    for index in 0..STEADY_TURNS {
        script.push(ScriptStep::DelayedSummary {
            text: marker(index),
            delay: TURN_DELAY,
        });
    }
    let mut server = server(script);
    let mut child = PerfChild::spawn(
        bin,
        &server.address,
        roots,
        fixture::CONFIG,
        &fixture::args(dialect, &connections, &workspace),
    );
    // The front is up once it has said anything at all; a front that says
    // nothing until it is asked is given the benefit of the doubt.
    let _ = child.out.wait_for_any(Duration::from_secs(15));
    thread::sleep(Duration::from_millis(500));

    // (d) idle CPU of the line front, where this host measures a live child.
    match idle_cpu(&child) {
        Some(percent) => record.idle_cpu_line_pct = Some(percent),
        None => record.skipped.push(
            "idle CPU of the line front: this host has no per-process CPU \
             accounting"
                .to_owned(),
        ),
    }

    // (a) send → the first streamed event of the first message.
    let sent = child.send_line("how much revenue is there?");
    let first = child
        .out
        .wait_for(FIRST_ANSWER.as_bytes(), TURN_TIMEOUT)
        .expect("the first message's answer");
    record.first_event_ms = (first - sent).as_secs_f64() * 1000.0;

    // (b) the steady turns, and (f) the state bytes one of them wrote.
    let mut samples = Vec::with_capacity(STEADY_TURNS);
    for index in 0..STEADY_TURNS {
        let held = state_bytes(&child);
        let sent = child.send_line(&format!("turn {index}"));
        let seen = child
            .out
            .wait_for(marker(index).as_bytes(), TURN_TIMEOUT)
            .unwrap_or_else(|error| panic!("turn {index}: {error}"));
        samples.push(TurnSample {
            wall: seen - sent,
            delay: TURN_DELAY,
        });
        if index + 1 == STEADY_TURNS {
            record.disk_bytes_per_turn = settle(held, &child, index);
        }
    }
    let overheads: Vec<f64> = samples
        .iter()
        .map(|sample| overhead(sample).as_secs_f64() * 1000.0)
        .collect();
    record.overhead_p50_ms = record::percentile(&overheads, 50.0);
    record.overhead_p95_ms = record::percentile(&overheads, 95.0);

    child.exit();
    server.join();

    // (c) request bytes per turn, from the mock's own captured bodies. The
    // first message is a turn like any other, so every body counts.
    let bodies = server.bodies.lock().unwrap().clone();
    let sizes: Vec<f64> = bodies.iter().map(|body| body.len() as f64).collect();
    assert!(
        sizes.len() >= STEADY_TURNS,
        "the mock served {} requests for {} turns",
        sizes.len(),
        STEADY_TURNS + 1
    );
    record.request_bytes_median = record::median(&sizes);
}

/// The bytes the run's own state directory holds: the session root (job
/// records and rewind) plus the state database and its write-ahead journal.
fn state_bytes(child: &PerfChild) -> u64 {
    let state = &child.roots.config_root;
    record::tree_bytes(&child.roots.session_root)
        + record::tree_bytes(&state.join("state.sqlite3"))
        + record::tree_bytes(&state.join("state.sqlite3-wal"))
        + record::tree_bytes(&state.join("state.sqlite3-shm"))
}

/// The state bytes the turn at `index` added: the directory is re-read after the
/// store has had a moment to finish its own writes.
fn settle(held: u64, child: &PerfChild, index: usize) -> f64 {
    thread::sleep(Duration::from_millis(400));
    let after = state_bytes(child);
    assert!(
        after >= held,
        "turn {index} lost state bytes ({held} → {after})"
    );
    (after - held) as f64
}

/// The harness's own self-check: one turn whose provider delay is known, whose
/// measured overhead must therefore be far below that delay. Remove the
/// subtraction in [`overhead`] and this fails, which is the point of it.
pub(crate) fn self_check(bin: &Path, dialect: &Dialect) {
    const DELAY: Duration = Duration::from_millis(400);
    let roots = Roots::new("self-check");
    let connections = fixture::sqlite_profile(&roots.config_root);
    let workspace = connection_workspace(&roots);
    let mut server = server(vec![
        // A warm-up turn first: the session's first turn creates the job and is
        // not what a steady-turn figure is about.
        ScriptStep::Summary("perf-warm-turn".to_owned()),
        ScriptStep::DelayedSummary {
            text: SELF_CHECK_ANSWER.to_owned(),
            delay: DELAY,
        },
    ]);
    let mut child = PerfChild::spawn(
        bin,
        &server.address,
        roots,
        fixture::CONFIG,
        &fixture::args(dialect, &connections, &workspace),
    );
    let _ = child.out.wait_for_any(Duration::from_secs(15));
    child.send_line("a warm-up turn");
    child
        .out
        .wait_for(b"perf-warm-turn", TURN_TIMEOUT)
        .expect("the warm-up turn's answer");
    let sent = child.send_line("a turn with no work of its own");
    let seen = child
        .out
        .wait_for(SELF_CHECK_ANSWER.as_bytes(), TURN_TIMEOUT)
        .expect("the self-check's answer");
    let sample = TurnSample {
        wall: seen - sent,
        delay: DELAY,
    };
    child.exit();
    server.join();

    let measured = overhead(&sample).as_secs_f64() * 1000.0;
    let delay = DELAY.as_secs_f64() * 1000.0;
    assert!(
        measured < delay / 2.0,
        "a turn whose provider held {delay:.0} ms measured {measured:.0} ms of \
         overhead: the provider's own delay was not subtracted, so every \
         steady-turn figure this harness reports would include it"
    );
}
