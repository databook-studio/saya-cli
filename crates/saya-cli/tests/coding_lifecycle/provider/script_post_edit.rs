use serde_json::json;

use super::render::{answer, tagged_tool, tool};

pub(super) fn response(actor: &str, stage: usize, mode: u8) -> String {
    let _ = mode;
    if actor == "parent" {
        return match stage {
            0 => tool(
                "helpers",
                json!({
                    "description":"apply the assigned workspace files",
                    "prompt":"write for child-post-edit: apply the assigned workspace files",
                    "subagent_type":"general",
                    "agent_id":"child-post-edit"
                }),
            ),
            _ => answer("native post-edit fixture finished"),
        };
    }

    match (mode, stage) {
        (30, 0) => tool("read", json!({"path":"tracked.txt"})),
        (30, 1) => tool(
            "write",
            json!({"path":"new.txt","content":"created by write\n"}),
        ),
        (30, 2) => tool(
            "edit",
            json!({
                "path":"tracked.txt",
                "edits":[
                    {"old_text":"original","new_text":"updated"},
                    {"old_text":"\n","new_text":"\n"}
                ]
            }),
        ),
        (30, 3) => tagged_tool(
            "post-write-replace",
            "edit",
            json!({
                "path":"new.txt",
                "old_text":"created by write",
                "new_text":"created and edited"
            }),
        ),
        (31 | 33, 0) => tool("write", json!({"path":"new.txt","content":"initial\n"})),
        (31 | 33, 1) => tool(
            "edit",
            json!({
                "path":"new.txt",
                "old_text":"server",
                "new_text":"followup"
            }),
        ),
        (32, 0) => tool(
            "write",
            json!({"path":"new.txt","content":"committed despite refusal\n"}),
        ),
        (32, 1) => tool(
            "write",
            json!({"path":"followup.txt","content":"must not run\n"}),
        ),
        (34 | 35, 0) => tool(
            "write",
            json!({"path":"new.txt","content":"known commit survives observer failure\n"}),
        ),
        _ => answer("post-edit child finished"),
    }
}
