#[cfg(test)]
use saya_types::redact_counted;
#[cfg(test)]
use serde_json::Value;

mod boundary;
mod completions;
mod execution;
mod failure_summary;
mod validity;

pub(super) use super::tool_batch::{complete_tool, execute_batch};
pub(super) use super::tool_shape::tool_message;
pub use super::tool_shape::{
    MAX_TOOL_MESSAGE_BYTES, ShapedToolResult, shape_tool_result, tool_message_cap,
};
pub(super) use boundary::floor_boundary;
#[cfg(test)]
pub(super) use execution::execute;
pub(super) use execution::{MAX_CONCURRENT_TOOL_CALLS, execute_with_context, execute_with_outcome};
#[cfg(test)]
use failure_summary::MAX_FAILURE_REASON_CHARS;
pub(super) use validity::invalid_reason;

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
