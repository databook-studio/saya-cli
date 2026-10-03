//! Model-facing tool-result shaping with bounded serialization.
//!
//! The serializer stops at the assigned cap, rather than first allocating a
//! complete JSON string. Redaction and its explanatory note are then fitted
//! back into that same cap, so the returned text is always the exact bounded
//! provider view.

use crate::ChatMessage;
use saya_types::redact_counted;
use serde_json::Value;

use super::tools::floor_boundary;

/// Absolute cap for one model-facing tool result.
pub const MAX_TOOL_MESSAGE_BYTES: usize = 65_536;
const TRUNCATION_MARKER: &str = "...[truncated]";

/// The exact model-facing form of one tool result.
pub struct ShapedToolResult {
    pub text: String,
    pub truncated: bool,
    pub redactions: usize,
}

/// Applies the individual hard cap to an assigned per-call byte budget.
pub fn tool_message_cap(byte_budget: usize) -> usize {
    byte_budget.min(MAX_TOOL_MESSAGE_BYTES)
}

/// Shapes `value` for a direct caller whose assigned cap is `byte_budget`.
pub fn shape_tool_result(value: &Value, byte_budget: usize) -> ShapedToolResult {
    shape_tool_result_at_cap(value, tool_message_cap(byte_budget))
}

pub(super) fn shape_tool_result_at_cap(value: &Value, cap: usize) -> ShapedToolResult {
    let (content, serialized_truncated) = bounded_json(value, cap);
    let (content, redactions) = redact_counted(&content);
    let decorated = if redactions > 0 {
        format!("{}\n\n{content}", redaction_note(redactions))
    } else {
        content
    };
    let truncated = serialized_truncated || decorated.len() > cap;
    ShapedToolResult {
        text: fit_text(&decorated, cap),
        truncated,
        redactions,
    }
}

pub(super) fn tool_message(id: String, result: Value, byte_budget: usize) -> (ChatMessage, bool) {
    let shaped = shape_tool_result(&result, byte_budget);
    (
        ChatMessage {
            role: "tool".into(),
            content: shaped.text,
            tool_calls: Vec::new(),
            tool_call_id: Some(id),
        },
        shaped.truncated,
    )
}

fn redaction_note(count: usize) -> String {
    format!(
        "[saya: {count} secret-shaped value(s) in this result were replaced with \
         [redacted]; the source is unchanged — do not write [redacted] back]"
    )
}

fn bounded_json(value: &Value, cap: usize) -> (String, bool) {
    let mut writer = BoundedWriter::new(cap);
    let written = serde_json::to_writer(&mut writer, value).is_ok();
    let text = writer.into_string();
    if written {
        (text, false)
    } else {
        (truncated_text(&text, cap), true)
    }
}

fn fit_text(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        text.to_owned()
    } else {
        truncated_text(text, cap)
    }
}

fn truncated_text(text: &str, cap: usize) -> String {
    let marker = &TRUNCATION_MARKER[..cap.min(TRUNCATION_MARKER.len())];
    let head = floor_boundary(text, cap.saturating_sub(marker.len()));
    format!("{}{marker}", &text[..head])
}

struct BoundedWriter {
    bytes: Vec<u8>,
    cap: usize,
}

impl BoundedWriter {
    fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(cap),
            cap,
        }
    }

    fn into_string(mut self) -> String {
        loop {
            match String::from_utf8(self.bytes) {
                Ok(text) => return text,
                Err(error) => {
                    let valid = error.utf8_error().valid_up_to();
                    self.bytes = error.into_bytes();
                    self.bytes.truncate(valid);
                }
            }
        }
    }
}

impl std::io::Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let room = self.cap.saturating_sub(self.bytes.len());
        let copied = room.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..copied]);
        Ok(copied)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
