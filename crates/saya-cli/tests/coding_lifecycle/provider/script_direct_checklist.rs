//! Scripted provider responses for the direct-mode checklist delta.
//!
//! Mode 112: the root writes a two-task list, makes three reads that change
//! nothing, then answers. The list is the same from the write on, so exactly
//! one request may carry it — the one the loop assembles after the write.

use super::super::script::render::{answer, tagged_tool, tool};
use serde_json::json;

/// Reads after the checklist write: every one of them is a turn whose list
/// did not change.
const READS: usize = 3;

pub(super) fn response(actor: &str, stage: usize, _request: &str) -> String {
    if actor != "parent" {
        return answer("no child runs for the checklist-delta mode");
    }
    match stage {
        0 => tool(
            "checklist",
            json!({"tasks":[
                {"title":"parse records","status":"in_progress"},
                {"title":"write the report","status":"pending"}
            ]}),
        ),
        stage if stage <= READS => tagged_tool(
            &format!("call-read-{stage}"),
            "read",
            json!({"path":"tracked.txt"}),
        ),
        _ => answer("the root read the checkout without changing its list"),
    }
}
