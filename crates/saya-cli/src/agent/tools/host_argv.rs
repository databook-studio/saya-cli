//! Host-lane argv admission: the `run_command` lane refuses an oversized
//! `args` array before cloning it into owned strings and before journalling
//! it — the host side of the sandbox lane's
//! `json_argv_is_checked_before_string_cloning` gate.

use saya_agent::ToolError;
use saya_harness::runner::{MAX_ARG_BYTES, MAX_ARG_COUNT, MAX_ARGV_BYTES};

/// Validates raw JSON argv against the process-lane bounds before any
/// cloning into owned strings: count, per-item bytes, and aggregate bytes.
pub(crate) fn validate_host_json_argv(items: &[serde_json::Value]) -> Result<(), ToolError> {
    if items.len() > MAX_ARG_COUNT {
        return Err(ToolError::Runner(
            "run_command refused: too many arguments".to_owned(),
        ));
    }
    let mut total = 0usize;
    for item in items {
        let value = item.as_str().ok_or(ToolError::UnsupportedProperty)?;
        if value.len() > MAX_ARG_BYTES {
            return Err(ToolError::Runner(
                "run_command refused: an argument exceeds the per-argument byte limit".to_owned(),
            ));
        }
        total = total.saturating_add(value.len());
        if total > MAX_ARGV_BYTES {
            return Err(ToolError::Runner(
                "run_command refused: arguments exceed the aggregate byte limit".to_owned(),
            ));
        }
    }
    Ok(())
}
