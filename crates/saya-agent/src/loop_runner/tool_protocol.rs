//! Whole-message tool-call protocol checks that must precede every side action.

use crate::ToolCall;
use std::collections::HashSet;

/// Collection bounds the calls and bytes while the provider response is built.
/// Once complete, every caller validates these reply IDs before treating the
/// response as an assistant message. This set keeps borrowed IDs only.
pub(super) fn valid_ids(calls: &[ToolCall]) -> bool {
    let mut ids = HashSet::with_capacity(calls.len());
    calls
        .iter()
        .all(|call| !call.id.trim().is_empty() && ids.insert(call.id.as_str()))
}
