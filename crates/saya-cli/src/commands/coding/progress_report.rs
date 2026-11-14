//! The progress a job's loop reported, recorded once per run as one JSON
//! line and read back for `saya job show` / `saya job export`.
//!
//! Reporting is not authority: a run that cannot record its report keeps its
//! own outcome, and the read surface reports the absence plainly. The record
//! is append-only, so a resumed job's latest run is the line that counts.

use super::context;
use saya_agent::ProgressReport;
use serde_json::{Value, json};
use std::path::Path;

const FILE: &str = "progress.jsonl";
/// A report is a turn count and a small map of event kinds; this is far more
/// room than a real one needs and still bounds the read.
const MAX_BYTES: usize = 64 * 1024;

/// Appends one report: the latest turn's check while the job runs, and the
/// final summary when it ends. Best effort: reporting is not authority.
pub(super) fn record(records: &Path, report: &ProgressReport) {
    let check = report.check.map(|check| {
        json!({
            "turn": check.turn,
            "turns_without_progress": check.turns_without_progress,
            "limit": check.limit,
            "reset": check.reset,
            "last_progress_turn": check.last_progress_turn,
            "message": check.message(),
        })
    });
    let Ok(mut line) = serde_json::to_vec(&json!({
        "turns": report.turns,
        "last_progress_turn": report.last_progress_turn,
        "progress_events": report.progress_events,
        "check": check,
    })) else {
        return;
    };
    line.push(b'\n');
    let _ = context::append_line(&records.join(FILE), &line, MAX_BYTES);
}

/// The latest recorded report, or `null` when the job never recorded one.
pub(super) fn read(records: &Path) -> Value {
    let Ok(bytes) = context::read(&records.join(FILE), MAX_BYTES) else {
        return Value::Null;
    };
    let Some(line) = bytes
        .split(|byte| *byte == b'\n')
        .rev()
        .find(|line| !line.is_empty())
    else {
        return Value::Null;
    };
    serde_json::from_slice(line).unwrap_or_else(|_| json!({"state": "unknown"}))
}
