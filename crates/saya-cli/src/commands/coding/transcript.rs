//! The per-call transcript a durable tool row carries, and the job-wide budget
//! that bounds its bulky part. Every call records its compact facts — id,
//! duration and the arguments cut to a small bound — whatever the budget says.
//! Only the result body counts against the job's ceiling: when it is reached,
//! later calls keep their compact facts and record no body, and one marker row
//! names the first call whose body was dropped. The loop already bounded each
//! field and the payload as a whole; this module is the writer's own bound,
//! applied after redaction — which can lengthen a text — and against the record
//! framing the journal enforces.

#[cfg(all(test, unix))]
#[path = "transcript_tests.rs"]
mod transcript_tests;

use super::diagnostics;
use saya_agent::{
    MAX_TOOL_CALL_ID_BYTES, MAX_TOOL_TRANSCRIPT_FIELD_BYTES, ToolCallTranscript,
    bound_tool_transcript,
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The largest serialized record the journal accepts, framing byte included.
/// The writer must fit a record exactly: an over-long one is rejected whole.
pub(super) const MAX_RECORD_BYTES: usize = diagnostics::RECORD_BYTES - 1;

/// The most result-body content one job records. Only the bulky part counts:
/// once a job reaches it, later calls keep their compact facts and record no
/// result body, and one marker row names the first call whose body was dropped.
pub(super) const MAX_JOB_TRANSCRIPT_BYTES: u64 = 256 * 1024;

/// The longest arguments text every call records, whatever the result-body
/// budget says: enough to name the command the model sent, never a body.
pub(super) const MAX_COMPACT_ARGUMENT_BYTES: usize = 512;

/// The smallest field budget the fit loop tries before giving up on a result
/// body: below this a head-and-tail cut says too little to be worth the
/// record's bytes.
const MIN_FIT_FIELD_BYTES: usize = 256;

/// The reserved tool name and closed words of the marker row.
const MARKER_TOOL: &str = "transcript";
const MARKER_STATUS: &str = "truncated";
const MARKER_REASON: &str = "transcript_truncated";

/// One job's transcript budget: how much result-body content its journal has
/// recorded, how many calls it has seen, the first call whose body it dropped,
/// and whether the marker row has been written.
pub(super) struct TranscriptBudget {
    ceiling: u64,
    recorded: AtomicU64,
    calls: AtomicU64,
    dropped_from: AtomicU64,
    marker_written: AtomicBool,
}

impl TranscriptBudget {
    pub(super) fn new() -> Self {
        Self::with_ceiling(MAX_JOB_TRANSCRIPT_BYTES)
    }

    pub(super) fn with_ceiling(ceiling: u64) -> Self {
        Self {
            ceiling,
            recorded: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            dropped_from: AtomicU64::new(0),
            marker_written: AtomicBool::new(false),
        }
    }

    fn exhausted(&self) -> bool {
        self.recorded.load(Ordering::Acquire) >= self.ceiling
    }

    /// Attaches this call's compact transcript to `row` — its id, its duration
    /// and its arguments cut to [`MAX_COMPACT_ARGUMENT_BYTES`] — and its result
    /// body while the job's budget has room, redacted and re-bounded. Reports
    /// whether the compact transcript was recorded: only a row that cannot hold
    /// even that is left bare, because the row's metadata is the fact that must
    /// survive.
    pub(super) fn attach(&self, row: &mut Value, transcript: &ToolCallTranscript) -> bool {
        let call = self.calls.fetch_add(1, Ordering::AcqRel).saturating_add(1);
        let Some(object) = row.as_object_mut() else {
            return false;
        };
        object.insert("transcript".to_owned(), compact_value(transcript));
        if record_bytes(row) > MAX_RECORD_BYTES {
            if let Some(object) = row.as_object_mut() {
                object.remove("transcript");
            }
            return false;
        }
        if let Some(result) = transcript.result()
            && !self.fit_result(row, &saya_types::redact(result))
        {
            self.note_body_dropped(call);
        }
        true
    }

    /// Inserts the result body at the largest field budget the record and the
    /// job's remaining budget both allow, and reports whether it was recorded.
    fn fit_result(&self, row: &mut Value, result: &str) -> bool {
        if self.exhausted() {
            return false;
        }
        let mut field = MAX_TOOL_TRANSCRIPT_FIELD_BYTES;
        loop {
            let bounded = bound_tool_transcript(result, field);
            let len = bounded.len() as u64;
            let Some(transcript) = row
                .as_object_mut()
                .and_then(|object| object.get_mut("transcript"))
                .and_then(Value::as_object_mut)
            else {
                return false;
            };
            transcript.insert("result".to_owned(), json!(bounded));
            if record_bytes(row) <= MAX_RECORD_BYTES && self.record_body(len) {
                return true;
            }
            if let Some(transcript) = row
                .as_object_mut()
                .and_then(|object| object.get_mut("transcript"))
                .and_then(Value::as_object_mut)
            {
                transcript.remove("result");
            }
            if field <= MIN_FIT_FIELD_BYTES {
                return false;
            }
            field = (field / 2).max(MIN_FIT_FIELD_BYTES);
        }
    }

    /// Reserves `len` result-body bytes against the job's ceiling. The
    /// compare-exchange makes the reservation exact under concurrent calls.
    fn record_body(&self, len: u64) -> bool {
        let mut recorded = self.recorded.load(Ordering::Acquire);
        loop {
            let Some(next) = recorded.checked_add(len) else {
                return false;
            };
            if next > self.ceiling {
                return false;
            }
            match self.recorded.compare_exchange(
                recorded,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => recorded = actual,
            }
        }
    }

    /// Remembers the first call whose result body was dropped. Later callers
    /// leave the first ordinal in place.
    fn note_body_dropped(&self, call: u64) {
        let _ = self
            .dropped_from
            .compare_exchange(0, call, Ordering::AcqRel, Ordering::Acquire);
    }

    /// The marker row for the first caller to drop a result body, and `None`
    /// for every other caller: exactly one row says the job's result bodies
    /// stopped, which call lost its body first, and how much body was recorded
    /// before that.
    pub(super) fn take_marker(&self, job_id: &str, epoch: u64) -> Option<Value> {
        let dropped_from = self.dropped_from.load(Ordering::Acquire);
        if dropped_from == 0 || self.marker_written.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(
            json!({"job_id": job_id, "agent_id": null, "invocation_id": null, "epoch": epoch,
            "tool": MARKER_TOOL, "status": MARKER_STATUS, "reason": MARKER_REASON,
            "source": "actual_tool_loop",
            "result_bodies_dropped_from_call": dropped_from,
            "transcript_bytes": self.recorded.load(Ordering::Acquire)}),
        )
    }
}

/// The serialized size of one journal record.
fn record_bytes(row: &Value) -> usize {
    serde_json::to_vec(row).map_or(usize::MAX, |bytes| bytes.len())
}

/// The compact transcript every call records: every text redacted exactly like
/// the rest of the job's evidence, and the arguments bounded again after
/// redaction, because redaction replaces a secret with a longer marker.
fn compact_value(transcript: &ToolCallTranscript) -> Value {
    json!({
        "call_id": bound_tool_transcript(
            &saya_types::redact(transcript.call_id()),
            MAX_TOOL_CALL_ID_BYTES,
        ),
        "arguments": bound_tool_transcript(
            &saya_types::redact(transcript.arguments()),
            MAX_COMPACT_ARGUMENT_BYTES,
        ),
        "duration_ms": transcript.duration_ms(),
    })
}
