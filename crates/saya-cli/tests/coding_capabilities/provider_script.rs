//! The capability scripts: which tool call or answer each actor receives at
//! each stage. The direct root hands work to helpers through one synchronous
//! `helpers` call per helper and reads its own working checkout with the
//! standard workspace tools; the read-only database tools are the standard
//! `schema_discovery` and `bounded_sql_query`, addressed by `connection`.

use super::sse;
use serde_json::{Value, json};

pub(super) fn response(actor: &str, stage: usize, mode: u8, request: &str) -> String {
    if mode == 36 {
        if actor == "parent" {
            return match stage {
                0 => helpers("call-task", "write for child-stage2", "child-stage2"),
                _ => answer("stage2 native journey completed without apply-back"),
            };
        }
        return match stage {
            0 => batch(&[
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
            // The edit of an existing file carries the digest the read proved.
            3 => tagged_tool(
                "stage2-multi-edit",
                "edit",
                json!({
                    "path":"tracked.txt",
                    "expected_digest":"2fdd50bdcab473e6cf6ad63952d5a597ff45f8303ee88ccc97b7136fc1342cd0",
                    "edits":[
                        {"old_text":"alpha-token","new_text":"beta-token"},
                        {"old_text":"beta-token","new_text":"gamma-token"}
                    ],
                }),
            ),
            4 => tagged_tool(
                "stage2-schema",
                "schema_discovery",
                json!({"connection":"scratch"}),
            ),
            5 => tagged_tool(
                "stage2-select",
                "bounded_sql_query",
                json!({"connection":"scratch","sql":"SELECT value FROM saya_sentinel"}),
            ),
            _ => answer("stage2 child complete"),
        };
    }
    if mode == 12 {
        if actor == "parent" {
            return match stage {
                0 => helpers("call-task", "write tracked.txt", "child-one"),
                _ => answer("the integrated root finished after its helper"),
            };
        }
        return match stage {
            // A read first: the write of an existing file carries the digest
            // the read proved, so the helper never overwrites blind.
            0 => tool("read", json!({"path":"tracked.txt"})),
            1 => tool(
                "write",
                json!({
                    "path":"tracked.txt",
                    "content":"child-assembled-content\n",
                    "expected_digest":"25718360e05d3c2d0963d1381e9dd4dae5fca789244ee4b9f861adcc0cc96218",
                }),
            ),
            _ => answer("the helper finished its tracked change"),
        };
    }
    if mode == 11 {
        return match stage {
            0 => tagged_tool("inspect-read", "read", json!({"path":"tracked.txt"})),
            1 => tagged_tool("inspect-list", "list", json!({})),
            2 => tagged_tool("inspect-missing", "read", json!({"path":"missing.txt"})),
            3 => tagged_tool("inspect-escape", "read", json!({"path":"../escape.txt"})),
            4 => tagged_tool(
                "inspect-write",
                "write",
                json!({"path":"tracked.txt","content":"candidate-only-content\n"}),
            ),
            _ => answer("the inspection root finished after writing its change"),
        };
    }
    // Mode 10: the two database probe helpers, merged into the shared working
    // checkout before the root's own database reads.
    if actor == "parent" {
        return match stage {
            0 => helpers("call-task-read", "read the approved database", "child-read"),
            1 => helpers(
                "call-task-tamper",
                "probe the database refusals",
                "child-tamper",
            ),
            2 => tagged_tool(
                "root-schema",
                "schema_discovery",
                json!({"connection": selected_profile(request)}),
            ),
            3 => tagged_tool(
                "root-read",
                "bounded_sql_query",
                json!({"connection":selected_profile(request),"sql":"SELECT value FROM saya_sentinel"}),
            ),
            4 => tagged_tool(
                "root-unselected",
                "bounded_sql_query",
                json!({"connection":"elsewhere","sql":"SELECT value FROM saya_sentinel"}),
            ),
            5 => tagged_tool(
                "root-missing-schema",
                "schema_discovery",
                json!({"connection":"zzmissing"}),
            ),
            _ => answer("database capability parent finished after merging its helpers"),
        };
    }
    if actor == "child-read" {
        return match stage {
            0 => tagged_tool(
                "child-schema",
                "schema_discovery",
                json!({"connection": selected_profile(request)}),
            ),
            1 => tagged_tool(
                "child-read",
                "bounded_sql_query",
                json!({"connection":selected_profile(request),"sql":"SELECT value FROM saya_sentinel"}),
            ),
            _ => answer("child database read complete"),
        };
    }
    if actor == "child-tamper" {
        return match stage {
            0 => tagged_tool(
                "child-sql-write",
                "bounded_sql_query",
                json!({"connection":selected_profile(request),"sql":"INSERT INTO saya_sentinel (value) VALUES ('tamper')"}),
            ),
            1 => tagged_tool(
                "child-sql-unselected",
                "bounded_sql_query",
                json!({"connection":"elsewhere","sql":"SELECT value FROM saya_sentinel"}),
            ),
            _ => answer("child database refusal probe complete"),
        };
    }
    answer("unexpected child actor finished")
}

/// The connection name the request approved: the first selected profile. The
/// root is approved for `scratch` and `zzmissing`, and only `scratch` resolves.
fn selected_profile(request: &str) -> String {
    let value: Value = serde_json::from_str(request).unwrap_or(Value::Null);
    value["tools"]
        .as_array()
        .and_then(|tools| {
            tools.iter().find_map(|tool| {
                tool["function"]["parameters"]["properties"]["connection"]["enum"]
                    .as_array()
                    .and_then(|names| names.first())
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
        })
        .unwrap_or_else(|| "scratch".to_owned())
}

/// One synchronous `helpers` call: the child's own task text carries
/// `write for <agent>`, which is how the transport attributes the child's
/// turns to that agent.
fn helpers(id: &str, description: &str, agent: &str) -> String {
    tagged_tool(
        id,
        "helpers",
        json!({
            "description": description,
            "prompt": format!("write for {agent}"),
            "subagent_type": "general",
            "agent_id": agent,
        }),
    )
}

/// Several tool calls in one assistant turn.
fn batch(calls: &[(&str, &str, Value)]) -> String {
    let wire = calls
        .iter()
        .enumerate()
        .map(|(index, (id, name, arguments))| {
            json!({"index":index,"id":id,"type":"function","function":{
                "name":name,"arguments":arguments.to_string()}})
        })
        .collect::<Vec<_>>();
    sse(json!({"choices":[{"index":0,"delta":{"tool_calls":wire},"finish_reason":"tool_calls"}]}))
}

fn tagged_tool(id: &str, name: &str, arguments: Value) -> String {
    sse(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
        "index":0,"id":id,"type":"function",
        "function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]}))
}

fn tool(name: &str, arguments: Value) -> String {
    tagged_tool(&format!("call-{name}"), name, arguments)
}

fn answer(text: &str) -> String {
    sse(json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}))
}
