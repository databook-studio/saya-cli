//! Bounded lifecycle timing spans for one coding job.
//!
//! The spans are durable lines in the job's private records directory, so a
//! finished run can say where its non-model time went without a second
//! measurement pass. Recording is best effort: a run never fails because its
//! timing could not be written, and the file is bounded by count and bytes.
//!
//! The model boundaries are derived from the transport spans the job already
//! records, on the same process-local clock the transport uses. No second
//! provider wrapper and no wall-clock coupling to the remote host: the
//! client-side stamp is the one the budget ledger observes.

use super::{CodingApplication, context};
use saya_store::SqliteStateStore;
use saya_types::JobId;
#[cfg(all(test, unix))]
use serde_json::Value;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// The private file one job's spans land in.
pub(crate) const FILE: &str = "lifecycle.jsonl";

/// The file's whole bound: at most 32 spans of a few hundred bytes each.
const MAX_BYTES: usize = 64 * 1024;
/// The most spans one run records. A lifecycle has a fixed, small set.
const MAX_SPANS: usize = 32;
/// The most distinct elapsed-note phases one run records. The notes are
/// per-turn timers (message assembly, prompt projection, persistence), so a
/// run's set is fixed and small even when its turn count is not.
pub(crate) const MAX_ELAPSED_PHASES: usize = 8;
/// The most summary lines one run writes: one per note phase when the
/// timeline ends, plus a late reading (a deferred probe finishing after it).
const MAX_SUMMARY_LINES: usize = 16;
/// The store page bound the derived model boundaries read.
const PAGE: usize = 100;
const MAX_PAGES: usize = 16;
/// A contested append is retried this many times before the span stays
/// pending for the next phase's flush.
const APPEND_ATTEMPTS: u32 = 3;
/// A summary is written once, when the timeline ends: a lost summary is lost
/// for good. The lock it contests is inherited by a forked child until it
/// execs, so a busy process can refuse the append for tens of milliseconds;
/// summaries retry over a wider, still bounded window.
const SUMMARY_APPEND_ATTEMPTS: u32 = 50;
const APPEND_RETRY: std::time::Duration = std::time::Duration::from_millis(1);

/// The process origin the binary marks before argument parsing. It is taken
/// once by the first job; later jobs in one process start their own timeline.
static PROCESS_ORIGIN: Mutex<Option<(Instant, Option<u64>)>> = Mutex::new(None);

/// Marks the process start and the transport clock's origin. Called once from
/// the binary entry, before argument parsing; a library caller gets the
/// application's own start instead.
pub(crate) fn mark_process_start() {
    let Ok(mut origin) = PROCESS_ORIGIN.lock() else {
        return;
    };
    if origin.is_none() {
        *origin = Some((Instant::now(), saya_agent::client_clock_stamp_ns()));
    }
}

fn take_origin() -> (Instant, Option<u64>) {
    PROCESS_ORIGIN
        .lock()
        .ok()
        .and_then(|mut origin| origin.take())
        .unwrap_or_else(|| (Instant::now(), saya_agent::client_clock_stamp_ns()))
}

struct Span {
    phase: &'static str,
    started_ms: u128,
    ended_ms: u128,
}

/// One phase's aggregate of the per-turn timers: how many readings it saw and
/// how long they took in total. Written as one summary line, never as a span
/// with a start and end, because the readings are not contiguous.
struct ElapsedNote {
    phase: &'static str,
    total_ms: u128,
    calls: u32,
    /// The aggregate last written, so the end-of-run write and a reading that
    /// arrives as it runs cannot record the same numbers twice.
    written: Option<(u32, u128)>,
}

struct State {
    records: Option<PathBuf>,
    current: Option<(&'static str, Instant)>,
    spans: Vec<Span>,
    written: usize,
    elapsed: Vec<ElapsedNote>,
    summary_lines: usize,
}

/// One job's lifecycle timeline.
pub(crate) struct LifecycleTimeline {
    origin: Instant,
    clock_at_origin: Option<u64>,
    state: Mutex<State>,
    finished: AtomicBool,
}

impl LifecycleTimeline {
    /// A timeline from the process start when the binary marked one, or from
    /// now for a library caller.
    pub(crate) fn new() -> Arc<Self> {
        let (origin, clock_at_origin) = take_origin();
        Arc::new(Self {
            origin,
            clock_at_origin,
            state: Mutex::new(State {
                records: None,
                current: Some(("app_open", origin)),
                spans: Vec::new(),
                written: 0,
                elapsed: Vec::new(),
                summary_lines: 0,
            }),
            finished: AtomicBool::new(false),
        })
    }

    /// Ends the current phase and starts `phase`. Cheap and synchronous: a
    /// span is written as soon as the job's records directory is known.
    pub(crate) fn enter(&self, phase: &'static str) {
        let at = Instant::now();
        let mut state = self.lock();
        if let Some((current, started)) = state.current.replace((phase, at)) {
            self.push(&mut state, current, started, at);
        }
        self.flush(&mut state);
    }

    /// Adds one bounded per-turn timer reading to `phase`'s aggregate. The
    /// aggregates are written once, when the timeline ends; a reading that
    /// arrives after that (a deferred probe finishing late) is written on its
    /// own, still bounded by the phase and line caps.
    pub(crate) fn note_elapsed(&self, phase: &'static str, elapsed: Duration) {
        let ms = elapsed.as_millis();
        let mut state = self.lock();
        let existing = state.elapsed.iter().position(|note| note.phase == phase);
        match existing {
            Some(index) => {
                let note = &mut state.elapsed[index];
                note.total_ms = note.total_ms.saturating_add(ms);
                note.calls = note.calls.saturating_add(1);
            }
            None if state.elapsed.len() < MAX_ELAPSED_PHASES => {
                state.elapsed.push(ElapsedNote {
                    phase,
                    total_ms: ms,
                    calls: 1,
                    written: None,
                });
            }
            // Past the phase bound the reading is dropped: the summaries stay
            // a fixed-size set however many phases a future caller names.
            None => return,
        }
        // A reading after the timeline ended (a deferred probe finishing late)
        // is still written, so a run that never waited for it is not silent
        // about what it cost.
        if self.finished.load(Ordering::Acquire) {
            self.append_summary(&mut state, phase);
        }
    }

    /// Ends the current phase without starting another, and writes the
    /// elapsed-note summaries.
    pub(crate) fn end(&self) {
        let at = Instant::now();
        let mut state = self.lock();
        if let Some((phase, started)) = state.current.take() {
            self.push(&mut state, phase, started, at);
        }
        self.flush(&mut state);
        let phases: Vec<&'static str> = state.elapsed.iter().map(|note| note.phase).collect();
        for phase in phases {
            self.append_summary(&mut state, phase);
        }
    }

    /// Binds the job's private records directory and flushes every buffered
    /// span into it.
    pub(crate) fn attach(&self, records: &Path) {
        let mut state = self.lock();
        state.records = Some(records.to_path_buf());
        self.flush(&mut state);
    }

    /// Ends the last phase and records the derived model boundaries: the
    /// process-start-to-first-dispatch and last-answer-to-exit spans, read
    /// from the transport stamps already durable in the store.
    pub(crate) async fn finish(&self, store: &SqliteStateStore, job: &JobId) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        self.end();
        if let Some((first, last)) = transport_bounds(store, job).await {
            self.derived(first, last);
        }
    }

    fn derived(&self, first_ns: u64, last_ns: u64) {
        let Some(clock_start) = self.clock_at_origin else {
            return;
        };
        let first = u128::from(first_ns.saturating_sub(clock_start) / 1_000_000);
        let last = u128::from(last_ns.saturating_sub(clock_start) / 1_000_000);
        let exit = self.origin.elapsed().as_millis();
        let mut state = self.lock();
        state.spans.push(Span {
            phase: "pre_model",
            started_ms: 0,
            ended_ms: first,
        });
        state.spans.push(Span {
            phase: "model",
            started_ms: first,
            ended_ms: last,
        });
        state.spans.push(Span {
            phase: "post_model",
            started_ms: last,
            ended_ms: exit.max(last),
        });
        self.flush(&mut state);
    }

    fn push(&self, state: &mut State, phase: &'static str, started: Instant, ended: Instant) {
        if state.spans.len() >= MAX_SPANS {
            return;
        }
        state.spans.push(Span {
            phase,
            started_ms: started.saturating_duration_since(self.origin).as_millis(),
            ended_ms: ended.saturating_duration_since(self.origin).as_millis(),
        });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn flush(&self, state: &mut State) {
        if state.records.is_none() {
            return;
        }
        while state.written < state.spans.len() && state.written < MAX_SPANS {
            let line = json!({
                "phase": state.spans[state.written].phase,
                "started_ms": state.spans[state.written].started_ms,
                "ended_ms": state.spans[state.written].ended_ms,
            });
            if !self.append(state, &line, APPEND_ATTEMPTS) {
                return;
            }
            state.written += 1;
        }
    }

    /// Writes one elapsed-note phase's current aggregate, once per phase at
    /// the end and once more for a reading that arrived after it. The line
    /// count is bounded, so a late stream of readings cannot grow the file
    /// past its fixed summary set.
    fn append_summary(&self, state: &mut State, phase: &'static str) {
        if state.summary_lines >= MAX_SUMMARY_LINES {
            return;
        }
        let Some(index) = state.elapsed.iter().position(|note| note.phase == phase) else {
            return;
        };
        let note = &state.elapsed[index];
        if note.written == Some((note.calls, note.total_ms)) {
            return;
        }
        let line = json!({
            "phase": note.phase,
            "summary": true,
            "calls": note.calls,
            "total_ms": note.total_ms,
        });
        let aggregate = (note.calls, note.total_ms);
        if self.append(state, &line, SUMMARY_APPEND_ATTEMPTS) {
            state.elapsed[index].written = Some(aggregate);
            state.summary_lines += 1;
        }
    }

    /// Appends one line to the records file, retrying a moment's lock
    /// contention a bounded few times. Timing is not authority either way — a
    /// directory that stays unavailable simply records fewer lines, and the
    /// run continues.
    fn append(&self, state: &State, line: &serde_json::Value, attempts: u32) -> bool {
        let Some(records) = state.records.as_ref() else {
            return false;
        };
        let path = records.join(FILE);
        let Ok(bytes) = serde_json::to_vec(line) else {
            return false;
        };
        let mut attempt = 0;
        loop {
            match context::append_line(&path, &bytes, MAX_BYTES) {
                Ok(()) => return true,
                Err(_) if attempt < attempts => {
                    attempt += 1;
                    std::thread::sleep(APPEND_RETRY);
                }
                Err(_) => return false,
            }
        }
    }
}

/// The first dispatch and last answer stamps from the job's durable transport
/// spans, on the client clock. Bounded: at most `MAX_PAGES` pages are read.
async fn transport_bounds(store: &SqliteStateStore, job: &JobId) -> Option<(u64, u64)> {
    let mut after = None;
    let mut first: Option<u64> = None;
    let mut last: Option<u64> = None;
    for _ in 0..MAX_PAGES {
        let page = store
            .list_coding_transport_attempts(job, after, PAGE)
            .await
            .ok()?;
        for item in &page.items {
            if let Some(span) = item.span.as_ref() {
                let data = span.data();
                if first.is_none() {
                    first = data.dispatch_started_ns;
                }
                last = Some(last.map_or(data.finished_ns, |seen: u64| seen.max(data.finished_ns)));
            }
        }
        match page.next_cursor {
            Some(cursor) => after = Some(cursor),
            None => break,
        }
    }
    first.zip(last)
}

impl CodingApplication {
    /// Ends the current phase and starts `phase` on the pending job timeline.
    pub(super) fn lifecycle_enter(&self, phase: &'static str) {
        self.lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .enter(phase);
    }

    /// Hands the pending timeline to the job being created and leaves a fresh
    /// one for any later job on this application.
    pub(super) fn take_lifecycle(&self) -> Arc<LifecycleTimeline> {
        let mut held = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut *held, LifecycleTimeline::new())
    }
}

/// The recorded spans of one job, in write order.
#[cfg(all(test, unix))]
pub(crate) fn read(records: &Path) -> Vec<Value> {
    let Ok(bytes) = context::read(&records.join(FILE), MAX_BYTES) else {
        return Vec::new();
    };
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect()
}

#[cfg(all(test, unix))]
#[path = "lifecycle_tests.rs"]
mod tests;
