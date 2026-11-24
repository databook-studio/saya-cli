//! The chat session's performance harness: build + smoke here, the timed k-run
//! campaign later on a quiet machine.
//!
//! `ref/06-plan.md` §8 (PERF) states the measures and their bars; this binary
//! takes the measures, and `scripts/perf-compare.sh` reads them back and applies
//! the bars. Nothing here calls a paid provider: the provider is the scripted
//! mock on loopback, and the database is a SQLite file the run creates itself.
//!
//! What one run measures, per `ref/06-plan.md` §8:
//!
//! * (a) send → the first streamed event of the first message;
//! * (b) a steady turn's overhead beyond the scripted provider, p50/p95 over
//!   [`line::STEADY_TURNS`] turns, with the provider's own scripted delay
//!   subtracted (its measurement is the script this harness wrote);
//! * (c) request bytes per turn, from the mock server's own capture;
//! * (d) idle CPU of both fronts over a 60 s window — the line front on a pipe,
//!   the full-screen front on a pty at 100x30;
//! * (e) frames painted per second while 1 000 tokens stream, counted from the
//!   pty's byte stream (one cursor-visibility command per frame). **Not**
//!   measured: the milliseconds a single frame costs — that needs a timing
//!   inside `Terminal::draw`, which is production instrumentation this harness
//!   may not add. The interval between frames is reported in its place, and the
//!   compare script marks the per-frame bar as not asserted;
//! * (f) the state bytes a turn without edits adds (job records + rewind).
//!
//! Each run appends one JSON line to `target/perf/<label>.jsonl` and prints its
//! own measures; the medians of the k runs go to stdout at the end.
//!
//! Env: `SAYA_PERF_BIN` (the binary under test; default `CARGO_BIN_EXE_saya`),
//! `SAYA_PERF_K` (runs per invocation; default 5), `SAYA_PERF_LABEL` (the jsonl
//! name; default the binary's file stem).

#[path = "perf_chat/child.rs"]
mod child;
#[path = "perf_chat/cpu.rs"]
mod cpu;
#[path = "perf_chat/dialect.rs"]
mod dialect;
#[path = "perf_chat/fixture.rs"]
mod fixture;
#[path = "perf_chat/harness.rs"]
#[allow(dead_code)] // shared scripted-provider harness: these rows use part of it
mod harness;
#[path = "perf_chat/line.rs"]
mod line;
#[cfg(unix)]
#[path = "perf_chat/pty.rs"]
mod pty;
#[path = "perf_chat/record.rs"]
mod record;
#[path = "perf_chat/spawn.rs"]
mod spawn;
#[cfg(unix)]
#[path = "perf_chat/tui.rs"]
mod tui;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use record::RunRecord;

/// The idle window each front's CPU is measured over.
pub(crate) const IDLE_WINDOW: Duration = Duration::from_secs(60);
/// The runs one invocation takes unless `SAYA_PERF_K` says otherwise.
const DEFAULT_K: usize = 5;

/// The binary under test: the control arm names its own build here.
fn binary() -> PathBuf {
    std::env::var_os("SAYA_PERF_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_saya")))
}

/// How many runs this invocation takes.
fn runs() -> usize {
    std::env::var("SAYA_PERF_K")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|k| *k > 0)
        .unwrap_or(DEFAULT_K)
}

/// The jsonl label: the arm's own name, or the binary's file stem.
fn label(bin: &Path) -> String {
    std::env::var("SAYA_PERF_LABEL").unwrap_or_else(|_| {
        bin.file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "perf".to_owned())
    })
}

#[test]
#[ignore = "perf harness: k timed runs, minutes each; run on a quiet machine"]
fn the_chat_session_meets_the_plans_performance_measures() {
    let bin = binary();
    let label = label(&bin);
    let k = runs();
    assert!(
        bin.is_file(),
        "SAYA_PERF_BIN names no binary: {}",
        bin.display()
    );
    // Once per binary per invocation: the spelling this build documents for
    // the run's read-only posture, and whether it documents `--plain`.
    let dialect = dialect::Dialect::of(&bin);
    println!(
        "perf-chat {label}: {} launches with {} read-only (--plain: {})",
        bin.display(),
        dialect.posture.flag(),
        dialect.plain
    );
    let mut records = Vec::with_capacity(k);
    for run in 0..k {
        let mut record = RunRecord {
            label: label.clone(),
            bin: bin.display().to_string(),
            run,
            k,
            turns: line::STEADY_TURNS,
            provider_delay_ms: line::TURN_DELAY.as_secs_f64() * 1000.0,
            idle_window_s: IDLE_WINDOW.as_secs_f64(),
            posture_flag: dialect.posture.flag().to_owned(),
            plain_flag: dialect.plain,
            ..RunRecord::default()
        };
        line::measure(&bin, &dialect, &mut record);
        measure_surface(&bin, &dialect, &mut record);
        record::append(&label, &record);
        record::print_run(&label, &record);
        records.push(record);
    }
    record::print_medians(&label, &records);
}

/// (d) for the full-screen front and (e): a pty is a Unix facility, so a host
/// without one reports the measures as skipped rather than inventing them.
fn measure_surface(bin: &Path, dialect: &dialect::Dialect, record: &mut RunRecord) {
    #[cfg(unix)]
    tui::measure(bin, dialect, record, IDLE_WINDOW);
    #[cfg(not(unix))]
    {
        let _ = (bin, dialect);
        record.skipped.push(
            "idle CPU of the TUI and draws while streaming: a pseudo-terminal is \
             unix-only"
                .to_owned(),
        );
    }
}

/// The harness's own self-check: a turn whose provider delay is known must
/// measure an overhead far below it. Remove the subtraction in
/// `line::overhead` and this fails — which is exactly its job.
#[test]
#[ignore = "perf harness: cheap self-check, runs with the harness"]
fn a_turn_overhead_subtracts_the_providers_own_delay() {
    let bin = binary();
    line::self_check(&bin, &dialect::Dialect::of(&bin));
}
