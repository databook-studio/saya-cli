//! The full-screen front's measures: the CPU it burns while nobody is typing,
//! and how many frames it paints while a long answer streams.
//!
//! Both run on one pty session at the plan's 100x30: the idle window first
//! (nothing typed, the surface only keeping itself current), then a 1 000-token
//! answer the scripted provider streams one chunk at a time while the pty's byte
//! stream is counted for frames.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use crate::{
    cpu, fixture,
    harness::ScriptStep,
    line,
    pty::{FRAME_MARKERS, TuiChild},
    record::RunRecord,
    spawn::Roots,
};

/// The plan's stream length, and how slowly one chunk is served.
const TOKENS: usize = 1000;
const CHUNK_DELAY: Duration = Duration::from_millis(5);
/// The text the last streamed chunk carries: the end of the measured window.
const LAST_TOKEN: &str = "tok999";
/// How long the stream may take before the harness calls it a hang.
const STREAM_TIMEOUT: Duration = Duration::from_secs(120);

/// Measures (d) for the surface and (e), on one pty session of `bin`.
pub(crate) fn measure(bin: &Path, record: &mut RunRecord, idle_window: Duration) {
    let roots = Roots::new(&format!("tui-run{}", record.run));
    let connections = fixture::sqlite_profile(&roots.config_root);
    let workspace = line::connection_workspace(&roots);
    let mut server = line::server(vec![ScriptStep::StreamTokens {
        count: TOKENS,
        chunk_delay: CHUNK_DELAY,
    }]);
    let mut child = TuiChild::spawn(
        bin,
        &server.address,
        roots,
        fixture::CONFIG,
        &fixture::args(&connections, &workspace),
    );
    // The surface is drawn once the screen stops changing.
    child
        .settle(Duration::from_millis(600), Duration::from_secs(30))
        .expect("the surface paints");

    // (d) idle CPU: the window is measured by the OS's own accounting of the
    // child, and its output is drained throughout so it never blocks on a full
    // pty instead of idling.
    let before = cpu::cpu_time(child.pid);
    child
        .drain_until(Instant::now() + idle_window)
        .expect("the surface stays live through the idle window");
    let after = cpu::cpu_time(child.pid);
    match (before, after) {
        (Some(before), Some(after)) => {
            record.idle_cpu_tui_pct = Some(cpu::percent(after.saturating_sub(before), idle_window));
        }
        _ => record
            .skipped
            .push("idle CPU of the TUI: this host has no per-process CPU accounting".to_owned()),
    }

    // (e) frames painted while 1 000 tokens stream, counted from the pty's own
    // byte stream: one cursor-visibility command per `Terminal::draw`.
    let from = child.raw_len();
    let sent = child.send_line("stream a thousand tokens");
    let seen = child.wait_for_output(LAST_TOKEN, STREAM_TIMEOUT);
    let elapsed = Instant::now().saturating_duration_since(sent);
    seen.unwrap_or_else(|error| {
        panic!(
            "the streamed answer never finished: {error}; stderr captured: {:?}",
            String::from_utf8_lossy(&child.raw_since(from))
        )
    });
    let painted = frames(&child.raw_since(from));
    record.tokens_streamed = TOKENS;
    if painted == 0 {
        record
            .skipped
            .push("draws while streaming: the pty stream carried no frame marker".to_owned());
    } else {
        let seconds = elapsed.as_secs_f64().max(f64::EPSILON);
        record.draws_per_second = Some(painted as f64 / seconds);
        record.frame_interval_ms = Some(seconds * 1000.0 / painted as f64);
    }
    drop(child);
    server.join();
}

/// The frames a raw pty stream carries: one cursor-visibility command each.
fn frames(raw: &[u8]) -> usize {
    FRAME_MARKERS
        .iter()
        .map(|marker| {
            if raw.len() < marker.len() {
                return 0;
            }
            raw.windows(marker.len())
                .filter(|window| *window == *marker)
                .count()
        })
        .sum()
}
