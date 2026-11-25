//! Which tool results a later call makes unnecessary.
//!
//! A result can never be needed again when a later call replaces its content:
//! a read of a path followed by a read of the same path at the same or a wider
//! range, or by a successful write or edit of it; and a shell, search or
//! database result whose exact call ran again later. Everything else stays
//! whole until the history reaches its hard ceiling.

use super::render;
use crate::ChatMessage;
use serde_json::Value;
use std::ops::Range;

/// One call in the order the model issued it. Owned rather than borrowed from
/// `messages` so a pass can rewrite a result while the call list is live; only
/// an over-budget pass pays for the copy.
pub(super) struct Call {
    pub(super) group: usize,
    name: String,
    arguments: Value,
    id: String,
    pub(super) result_index: Option<usize>,
    result_is_error: bool,
}

impl Call {
    pub(super) fn is_read(&self) -> bool {
        self.name == "read"
    }

    fn is_write(&self) -> bool {
        matches!(self.name.as_str(), "write" | "edit" | "notebook_edit")
    }

    /// Old non-read output: anything that is neither a read nor a write.
    pub(super) fn is_old_output(&self) -> bool {
        !self.is_read() && !self.is_write()
    }

    fn path(&self) -> Option<&str> {
        (self.is_read() || self.is_write())
            .then(|| self.arguments.get("path").and_then(Value::as_str))
            .flatten()
    }

    fn ran_ok(&self) -> bool {
        self.result_index.is_some() && !self.result_is_error
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn arguments(&self) -> &Value {
        &self.arguments
    }
}

/// The calls of every group, in issue order.
pub(super) fn collect(messages: &[ChatMessage], groups: &[Range<usize>]) -> Vec<Call> {
    let mut calls = Vec::new();
    for (group, range) in groups.iter().enumerate() {
        let assistant = &messages[range.start];
        for tool in &assistant.tool_calls {
            let result_index = (range.start + 1..range.end)
                .find(|&index| messages[index].tool_call_id.as_deref() == Some(tool.id.as_str()));
            calls.push(Call {
                group,
                name: tool.name.clone(),
                arguments: tool.arguments.clone(),
                id: tool.id.clone(),
                result_index,
                result_is_error: result_index
                    .is_some_and(|index| render::is_error(&messages[index].content)),
            });
        }
    }
    calls
}

/// The later call that makes `position`'s result unnecessary, if any.
pub(super) fn superseded_by(calls: &[Call], position: usize) -> Option<&Call> {
    let earlier = &calls[position];
    calls
        .get(position + 1..)
        .unwrap_or(&[])
        .iter()
        .find(|later| {
            if !later.ran_ok() {
                return false;
            }
            if earlier.is_read() {
                let Some(path) = earlier.path() else {
                    return false;
                };
                if later.path() != Some(path) {
                    return false;
                }
                if later.is_write() {
                    return true;
                }
                later.is_read() && read_range(later).covers(&read_range(earlier))
            } else if earlier.is_write() {
                false
            } else {
                later.name == earlier.name && later.arguments == earlier.arguments
            }
        })
}

/// A line range; `None` end means the read had no count and covered the rest
/// of the file.
struct Lines {
    start: u64,
    end: Option<u64>,
}

impl Lines {
    fn covers(&self, earlier: &Lines) -> bool {
        self.start <= earlier.start
            && match (self.end, earlier.end) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(later), Some(earlier)) => later >= earlier,
            }
    }
}

fn read_range(call: &Call) -> Lines {
    let start = call
        .arguments
        .get("start_line")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let end = call
        .arguments
        .get("line_count")
        .and_then(Value::as_u64)
        .map(|count| start.saturating_add(count));
    Lines { start, end }
}
