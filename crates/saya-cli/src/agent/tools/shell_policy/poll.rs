//! The background read and kill calls.
//!
//! The first read of a run answers at once: the harness drains the unread
//! bytes and reports the status. A read of the same live run that follows a
//! read with no new output and no exit waits inside the call for new output or
//! the exit, backing off 30 s, then 60 s, then 120 s for further silent reads,
//! so a model that ignores the completion notice cannot turn every poll into a
//! full-price instant request. Any output or the exit resets the sequence, and
//! every wait is clamped to the job's remaining wall clock. A live run's
//! result still says the exit will arrive as a notice, not as the answer to
//! another poll.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use saya_agent::ToolError;
use saya_harness::runner::{ReadReport, StreamCapture, WorkspaceShell};
use serde_json::{Value, json};

use super::arguments::{background_id, validate_kill, validate_output};
use super::tool::ShellTool;

/// What a read of a live run says: the exit will arrive as a notice, not as
/// the answer to another poll.
const STILL_RUNNING: &str = "still running; you will be told when it exits";

/// How long one harness read waits before the tool looks at its output. The
/// harness read returns at the run's end, not at new output, so the tool takes
/// the budget in slices; 250 ms bounds how long a written line waits to be
/// seen while keeping the loop cheap.
const POLL_SLICE: Duration = Duration::from_millis(250);

/// The wait a repeated silent read takes. The first read of an id never
/// waits; the next waits 30 s, then 60 s, then 120 s for every later one.
const SILENT_READ_WAITS: [Duration; 3] = [
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(120),
];

/// The job's wall clock: when the shell was composed and how long the job may
/// run. Every silent-read wait is clamped to what remains, so a quiet build
/// cannot hold the tool past the job's own ceiling.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WallClock {
    started: Instant,
    limit: Duration,
}

impl WallClock {
    pub(crate) fn new(limit: Duration) -> Self {
        Self {
            started: Instant::now(),
            limit,
        }
    }

    /// What remains of the job's wall clock.
    fn remaining(&self) -> Duration {
        self.limit.saturating_sub(self.started.elapsed())
    }
}

/// The consecutive silent reads of every run this shell started. Interior
/// mutability because a tool call takes `&self`; a lock is never held across
/// an await.
pub(super) struct ReadBook {
    waits: [Duration; 3],
    silent: Mutex<HashMap<u64, u32>>,
}

impl Default for ReadBook {
    fn default() -> Self {
        Self::new(SILENT_READ_WAITS)
    }
}

impl ReadBook {
    pub(super) fn new(waits: [Duration; 3]) -> Self {
        Self {
            waits,
            silent: Mutex::new(HashMap::new()),
        }
    }

    /// How many consecutive silent reads `id` has had.
    fn silent(&self, id: u64) -> u32 {
        self.lock().get(&id).copied().unwrap_or(0)
    }

    /// Counts one silent read and returns the new count.
    fn silent_read(&self, id: u64) -> u32 {
        let mut runs = self.lock();
        let count = runs.entry(id).or_insert(0);
        *count = count.saturating_add(1);
        *count
    }

    /// Forgets a run that wrote, ended, or was killed: its next read starts a
    /// new count.
    fn forget(&self, id: u64) {
        self.lock().remove(&id);
    }

    /// The wait before a read that already has `silent` silent reads behind it.
    fn wait(&self, silent: u32) -> Duration {
        match silent {
            0 => Duration::ZERO,
            count => self.waits[(count as usize - 1).min(self.waits.len() - 1)],
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<u64, u32>> {
        self.silent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Whether a report carries bytes the model has not seen.
fn wrote(report: &ReadReport) -> bool {
    !report.stdout.text.is_empty() || !report.stderr.text.is_empty()
}

/// One run's account over two reads: the harness drains the unread bytes on
/// every read, so the bytes, the drop counts, and the truncation flags add up.
fn merge(first: ReadReport, second: ReadReport) -> ReadReport {
    ReadReport {
        id: second.id,
        status: second.status,
        stdout: merge_capture(first.stdout, second.stdout),
        stderr: merge_capture(first.stderr, second.stderr),
    }
}

fn merge_capture(first: StreamCapture, second: StreamCapture) -> StreamCapture {
    StreamCapture {
        text: first.text + &second.text,
        truncated: first.truncated || second.truncated,
        dropped_bytes: first.dropped_bytes.saturating_add(second.dropped_bytes),
        total_bytes: first.total_bytes.saturating_add(second.total_bytes),
    }
}

/// Waits inside one `shell_output` call. Returns the first report that carries
/// new output or a terminal status; otherwise returns the last report when the
/// budget is spent. The run's own cancellation ends the wait too, because the
/// harness read checks the job's token on every slice.
async fn wait_for_change(
    shell: &WorkspaceShell,
    id: u64,
    budget: Duration,
) -> Result<ReadReport, ToolError> {
    let deadline = Instant::now() + budget;
    let mut report: Option<ReadReport> = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let slice = remaining.min(POLL_SLICE);
        let next = shell
            .read_background(id, slice)
            .await
            .map_err(|error| ToolError::Runner(error.to_string()))?;
        let next = match report.take() {
            Some(previous) => merge(previous, next),
            None => next,
        };
        if wrote(&next) || next.status.is_terminal() || slice >= remaining {
            return Ok(next);
        }
        report = Some(next);
    }
}

impl ShellTool {
    /// Clamps one wait to the job's remaining wall clock, so a read cannot
    /// hold the tool past the job's own ceiling. A shell with no job clock
    /// (an interactive session) waits the backoff it was given.
    fn clamp_wait(&self, wait: Duration) -> Duration {
        match self.wall_clock {
            Some(clock) => wait.min(clock.remaining()),
            None => wait,
        }
    }

    /// Reads one background run: the output written since the last read and
    /// the run's status, waiting inside the call only when the previous read
    /// of this live run was silent.
    pub(super) async fn output_call(&self, arguments: &Value) -> Result<Value, ToolError> {
        if !self.background {
            return Err(ToolError::UnsupportedTool);
        }
        validate_output(arguments)?;
        let id = background_id(arguments.get("id"))?;
        let silent = self.reads.silent(id);
        let budget = self.clamp_wait(self.reads.wait(silent));
        let report = if budget.is_zero() {
            self.shell
                .read_background(id, Duration::ZERO)
                .await
                .map_err(|error| ToolError::Runner(error.to_string()))?
        } else {
            wait_for_change(&self.shell, id, budget).await?
        };
        // New output or the exit ends the sequence; a silent read extends it,
        // so the next read waits the next step of the table.
        if wrote(&report) || report.status.is_terminal() {
            self.reads.forget(id);
        } else {
            self.reads.silent_read(id);
        }
        let mut result = self.read_report(&report);
        if report.status.is_running() {
            result["note"] = Value::String(STILL_RUNNING.into());
        }
        Ok(result)
    }

    /// Ends one background run, whole process group and all.
    pub(super) async fn kill_call(&self, arguments: &Value) -> Result<Value, ToolError> {
        if !self.background {
            return Err(ToolError::UnsupportedTool);
        }
        validate_kill(arguments)?;
        let id = background_id(arguments.get("id"))?;
        let report = self
            .shell
            .kill_background(id)
            .await
            .map_err(|error| ToolError::Runner(error.to_string()))?;
        self.reads.forget(id);
        Ok(self.read_report(&report))
    }

    /// What one read reports: the status the shell recorded, the exit code
    /// where there is one, and the output since the last read, bounded for the
    /// model exactly as a foreground result is. `truncated` is served only
    /// when the output was cut.
    fn read_report(&self, report: &ReadReport) -> Value {
        let bounded = self.bound(&report.stdout, &report.stderr);
        let mut result = json!({
            "id": report.id,
            "status": report.status.as_str(),
            "exit_code": report.status.exit_code(),
            "stdout": bounded.stdout,
            "stderr": bounded.stderr,
            "truncated": report.truncated() || bounded.truncated,
        });
        crate::agent::tools::omit_default_flag(&mut result, "truncated", false);
        result
    }
}
