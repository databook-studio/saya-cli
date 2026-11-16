//! The bounded record of one tool call: the id the provider gave it, the
//! arguments the model sent, the result the tool returned and how long the call
//! took. The agent loop attaches it to the call's [`crate::ToolDiagnostic`],
//! and the durable writer stores it redacted and re-bounded.
//!
//! A coding job's tool evidence carries content deliberately, so every bound
//! is explicit and every field is cut
//! on a UTF-8 boundary with an elision marker between the retained head and
//! tail. The head/tail cut is the shape the runner's captured output uses
//! (`saya-harness/src/runner/output.rs`); it is reimplemented here because
//! `saya-harness` depends on this crate, so the dependency direction forbids
//! reaching up for it.
//!
//! These bounds keep one call's payload small: the transcript rides beside a
//! child's projected event — never inside it, since a projection carries no
//! content — in the same in-process message, whose own bound is the
//! coordinator's 8 KiB `MAX_CHILD_EVENT_BYTES`. The durable writer re-applies
//! every bound after redaction, which can lengthen a text.

/// The longest one recorded argument or result text may be, measured before
/// the record's own JSON escaping.
pub const MAX_TOOL_TRANSCRIPT_FIELD_BYTES: usize = 2 * 1024;

/// The longest a whole serialized transcript may be. A field's own bound is
/// measured on raw text, but a field travels JSON-escaped, so two maximal
/// fields can serialize to more than twice their bound; this bound is what the
/// payload actually has to satisfy, and the field budget is halved until it
/// does. Five KiB leaves the diagnostic the transcript rides beside — its
/// identity, its closed words, and the bounded reason text of a failed call —
/// inside the seven-KiB payload one child call carries.
pub const MAX_TOOL_TRANSCRIPT_BYTES: usize = 5 * 1024;

/// The longest recorded tool-call id.
pub const MAX_TOOL_CALL_ID_BYTES: usize = 128;

/// The smallest field budget the whole-transcript fit loop tries. A transcript
/// bounded to this fits [`MAX_TOOL_TRANSCRIPT_BYTES`] whatever the escaping, so
/// the loop always terminates on a transcript inside the bound.
const MIN_TOOL_TRANSCRIPT_FIELD_BYTES: usize = 128;

/// Stands between the retained head and tail of a bounded field.
pub const TOOL_TRANSCRIPT_ELISION_MARKER: &str = "...[elided]...";

/// The bounded record of one tool call: its id, the arguments the model sent,
/// the result the tool returned when there was one, and the call's duration.
/// Fields are private and every constructor bounds them, so no unbounded text
/// can reach a sink.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolCallTranscript {
    call_id: String,
    arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    duration_ms: u64,
}

impl ToolCallTranscript {
    /// Bounds each field to its own constant and the whole transcript to
    /// [`MAX_TOOL_TRANSCRIPT_BYTES`], halving the field budget while the
    /// serialized payload is over it — a bounded text can still serialize to
    /// twice its length once escaped. `result` is `None` for a call that
    /// produced no result the model could read — a call refused at batch
    /// admission, for instance.
    ///
    /// Public because it is the only way to build one and it admits nothing
    /// unbounded: what it returns is always inside these bounds. Attaching a
    /// transcript to a diagnostic stays crate-private to the loop.
    pub fn new(call_id: &str, arguments: &str, result: Option<&str>, duration_ms: u64) -> Self {
        let mut field = MAX_TOOL_TRANSCRIPT_FIELD_BYTES;
        loop {
            let transcript = Self::at_field_budget(call_id, arguments, result, duration_ms, field);
            let serialized =
                serde_json::to_vec(&transcript).map_or(usize::MAX, |bytes| bytes.len());
            if serialized <= MAX_TOOL_TRANSCRIPT_BYTES || field <= MIN_TOOL_TRANSCRIPT_FIELD_BYTES {
                return transcript;
            }
            field = (field / 2).max(MIN_TOOL_TRANSCRIPT_FIELD_BYTES);
        }
    }

    /// The transcript with every text bounded to `field`.
    fn at_field_budget(
        call_id: &str,
        arguments: &str,
        result: Option<&str>,
        duration_ms: u64,
        field: usize,
    ) -> Self {
        Self {
            call_id: bound_tool_transcript(call_id, MAX_TOOL_CALL_ID_BYTES),
            arguments: bound_tool_transcript(arguments, field),
            result: result.map(|text| bound_tool_transcript(text, field)),
            duration_ms,
        }
    }

    /// The id the provider gave this call.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// The arguments the model sent, serialized and bounded.
    pub fn arguments(&self) -> &str {
        &self.arguments
    }

    /// The result the tool returned, bounded; `None` when the call produced
    /// none.
    pub fn result(&self) -> Option<&str> {
        self.result.as_deref()
    }

    /// How long the call took, in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
}

/// Keeps both ends of `text` inside `cap` bytes with
/// [`TOOL_TRANSCRIPT_ELISION_MARKER`] between them, cutting on UTF-8
/// boundaries. A text that already fits is returned unchanged, and a cap too
/// small to hold the marker yields the empty string rather than a partial one.
pub fn bound_tool_transcript(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_owned();
    }
    let keep = cap.saturating_sub(TOOL_TRANSCRIPT_ELISION_MARKER.len());
    if keep < 2 {
        return String::new();
    }
    let head = floor_boundary(text, keep / 2);
    let tail = ceil_boundary(text, text.len() - (keep - head));
    format!(
        "{}{}{}",
        &text[..head],
        TOOL_TRANSCRIPT_ELISION_MARKER,
        &text[tail..]
    )
}

/// The largest index at or below `end` that starts a character.
fn floor_boundary(text: &str, mut end: usize) -> usize {
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// The smallest index at or above `start` that starts a character.
fn ceil_boundary(text: &str, mut start: usize) -> usize {
    while !text.is_char_boundary(start) {
        start += 1;
    }
    start
}

#[cfg(test)]
#[path = "tool_transcript_tests.rs"]
mod tool_transcript_tests;
