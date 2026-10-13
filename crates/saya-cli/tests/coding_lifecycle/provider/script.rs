//! The per-mode scripts: which tool call or answer each actor receives at
//! each stage, as the SSE bodies the transport sends.

use serde_json::{Value, json};

pub(super) fn response(actor: &str, stage: usize, injection: bool, mode: u8) -> String {
    if actor == "parent" {
        if mode == 7 {
            return match stage {
                0 => tool(
                    "delegate",
                    json!({"tasks":[{"agent_id":"child-c1","task":"write for child-c1","write_paths":["only.txt"]},{"agent_id":"child-c2","task":"write for child-c2","write_paths":["only.txt"]}]}),
                ),
                1 => tool(
                    "delegate",
                    json!({"tasks":[{"agent_id":"child-one","task":"write for child-one","write_paths":["only.txt"]}]}),
                ),
                2 => tool("await_children", json!({})),
                _ => answer("one-path parent finished"),
            };
        }
        if mode == 8 {
            return match stage {
                0 => tool(
                    "delegate",
                    json!({"tasks":[{"agent_id":"child-one","task":"write for child-one","write_paths":["only.txt"]}]}),
                ),
                1 => tool("await_children", json!({})),
                2 => tool(
                    "delegate",
                    json!({"tasks":[{"agent_id":"child-second","task":"write for child-second","write_paths":["only.txt"]}]}),
                ),
                _ => answer("one-path parent finished"),
            };
        }
        if mode == 6 {
            return match stage {
                0 => tool("cancel_child", json!({"agent_id":"ghost"})),
                1 => tool(
                    "message_child",
                    json!({"agent_id":"ghost","message":"steer the ghost"}),
                ),
                2 => tool("coding_status", json!({})),
                _ => answer("parity parent finished after the unregistered-child refusals"),
            };
        }
        return match stage {
            0 if mode == 3 || mode == 5 => {
                let count = if mode == 3 { 24 } else { 2 };
                let tasks = (0..count).map(|index| {
                    let id = if mode == 3 { format!("child-{index:02}") } else if index == 0 { "child-a".into() } else { "child-b".into() };
                    let dependencies = if index == 0 { Vec::new() } else if mode == 3 { vec![format!("child-{:02}", index - 1)] } else { vec!["child-a".into()] };
                    json!({"agent_id":id,"task":format!("write for {id}"),"write_paths":["shared.txt","rename-before.txt","rename-after.txt"],"dependencies":dependencies})
                }).collect::<Vec<_>>();
                tool("delegate", json!({"tasks":tasks}))
            }
            0 => tool(
                "delegate",
                json!({"tasks":[{"agent_id":"child-a","task":"write for child-a","write_paths":["new-a.txt"]},{"agent_id":"child-b","task":"write for child-b","write_paths":["new-b.txt"]}]}),
            ),
            1 => tool("await_children", json!({})),
            _ => answer("native parent finished after joining children"),
        };
    }
    if mode == 1 && stage == 0 {
        return tool("coding_check", json!({"profile":"lint"}));
    }
    if mode == 2 && stage == 0 {
        return tool(
            "coding_command_start",
            json!({"program":"sleep","argv":["20"],"timeout_seconds":30}),
        );
    }
    if (mode == 3 || mode == 5) && stage == 0 {
        return tool(
            "workspace_write",
            json!({"path":"shared.txt","content":format!("{actor}\n")}),
        );
    }
    if (mode == 3 && actor == "child-00" || mode == 5 && actor == "child-a") && stage == 1 {
        return tool(
            "workspace_write",
            json!({"path":"rename-before.txt","content":"stable rename bytes\n"}),
        );
    }
    if mode == 3 || mode == 5 {
        return answer("child work complete");
    }
    if mode != 0 {
        return answer("child command evidence obtained");
    }
    if injection {
        match stage {
            0 => return tool("coding_skills", json!({"name":"poison"})),
            1 => {
                return tool(
                    "delegate",
                    json!({"tasks":[{"agent_id":"escape","task":"grant new root","write_paths":["outside.txt"]}]}),
                );
            }
            2 => {
                return tool(
                    "workspace_write",
                    json!({"path":"outside.txt","content":"must not exist"}),
                );
            }
            _ => {}
        }
    }
    if stage == if injection { 3 } else { 0 } {
        let path = if actor == "child-a" {
            "new-a.txt"
        } else {
            "new-b.txt"
        };
        tool(
            "workspace_write",
            json!({"path":path,"content":format!("native {actor}\n")}),
        )
    } else {
        answer("child work complete")
    }
}
fn tool(name: &str, args: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
