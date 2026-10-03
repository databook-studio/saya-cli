//! Whole-message tool-call protocol checks that must precede every side action.

use crate::ToolCall;
use std::collections::HashSet;

/// IDs identify provider tool replies, so each completed assistant message
/// needs one nonblank, unique ID per call before the loop can do anything with
/// that batch. The provider collector already caps the call count and bytes;
/// this set keeps borrowed IDs only.
pub(super) fn valid_ids(calls: &[ToolCall]) -> bool {
    let mut ids = HashSet::with_capacity(calls.len());
    calls
        .iter()
        .all(|call| !call.id.trim().is_empty() && ids.insert(call.id.as_str()))
}
