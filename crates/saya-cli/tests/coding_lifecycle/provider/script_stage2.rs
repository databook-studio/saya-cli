use super::render::{answer, tagged_tool, tool, tools};
use serde_json::json;

pub(super) fn response(actor: &str, stage: usize, request: &str) -> String {
    let _ = request;
    if actor == "parent" {
        return match stage {
            0 => tool(
                "helpers",
                json!({"description":"write the assigned files","prompt":"write for child-stage2","subagent_type":"general","agent_id":"child-stage2"}),
            ),
            // The retired lifecycle calls (`coding_status`,
            // `coding_prepare_candidate`, `coding_inspect`) have no
            // replacement: the owner prepares at the end of the run and the
            // operator reviews it, so the root's remaining turns are plain
            // reads of the shared checkout.
            1 => tagged_tool("stage2-status", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool("stage2-prepare", "read", json!({"path":"tracked.txt"})),
            3 => tagged_tool(
                "stage2-inspect-candidate",
                "read",
                json!({"path":"tracked.txt"}),
            ),
            _ => answer("stage2 native journey completed without apply-back"),
        };
    }
    match stage {
        0 => tools(&[
            ("stage2-read-a", "read", json!({"path":"read-a.txt"})),
            ("stage2-read-b", "read", json!({"path":"read-b.txt"})),
            ("stage2-read-c", "read", json!({"path":"read-c.txt"})),
            ("stage2-read-d", "read", json!({"path":"read-d.txt"})),
        ]),
        1 => tagged_tool(
            "stage2-range",
            "read",
            json!({"path":"large.txt","start_line":900,"line_count":2}),
        ),
        2 => tagged_tool("stage2-tracked-read", "read", json!({"path":"tracked.txt"})),
        3 => tagged_tool(
            "stage2-multi-edit",
            "edit",
            json!({"path":"tracked.txt","edits":[
                {"old_text":"alpha-token","new_text":"beta-token"},
                {"old_text":"beta-token","new_text":"gamma-token"}
            ]}),
        ),
        // The retired `coding_command_start` / `coding_command_wait` pair is one
        // synchronous `shell` call.
        4 => tagged_tool(
            "stage2-command-start",
            "shell",
            json!({"command":"sleep 1","description":"run the command","sandbox":true}),
        ),
        5 => tagged_tool(
            "stage2-command-natural-wait",
            "shell",
            json!({"command":"true","description":"observe the command","sandbox":true}),
        ),
        6 => tagged_tool(
            "stage2-command-repeat-wait",
            "shell",
            json!({"command":"true","description":"observe the command again","sandbox":true}),
        ),
        7 => tagged_tool(
            "stage2-schema",
            "schema_discovery",
            json!({"connection":"scratch"}),
        ),
        8 => tagged_tool(
            "stage2-select",
            "bounded_sql_query",
            json!({"connection":"scratch","sql":"SELECT value FROM saya_sentinel"}),
        ),
        _ => answer("stage2 child complete"),
    }
}
