//! The scripted SSE bodies: which tool call or answer each actor receives at
//! each stage. Sentinels are secret-shaped markers planted where the
//! diagnostics contract says nothing may leak: model-supplied tool arguments,
//! tool result bodies, and provider reasoning.

use serde_json::{Value, json};

/// Planted in a delegate task argument (never executed content).
pub(crate) const ARG_SENTINEL: &str = "T4SK-ARG-SENTINEL-a1b2c3d4";
/// Planted in a file body the child successfully writes (tool argument and,
/// via the next turn's tool message, result-side content).
pub(crate) const RESULT_SENTINEL: &str = "R3SULT-BODY-SENTINEL-9d8e7f01";
/// Planted in the provider's reasoning stream for the parent's first turn.
pub(crate) const REASONING_SENTINEL: &str = "R34SONING-SENTINEL-5e6f7a02";

/// A protocol-valid unknown name whose content must never be persisted.
pub(crate) fn unknown_tool_name() -> String {
    "unknown_SECRET_SENTINEL".into()
}

/// The file the transcript child writes.
pub(crate) const TRANSCRIPT_PATH: &str = "transcript-a.txt";
/// The command the transcript child runs through its own `shell`.
pub(crate) const TRANSCRIPT_COMMAND: &str = "echo";
/// The stdout that command produces; the recorded result must carry it.
pub(crate) const TRANSCRIPT_STDOUT: &str = "TRANSCRIPT-SHELL-OK-3c8d";
/// The credential-shaped value planted in a recorded argument. Fixture style:
/// the same shapes the existing redaction tests use, never a real secret.
pub(crate) const TRANSCRIPT_SECRET_ARGUMENT: &str = "password=hunter2";
/// The credential-shaped value planted in a command whose stdout echoes it, so
/// the recorded *result* carries one too.
pub(crate) const TRANSCRIPT_SECRET_RESULT: &str = "Authorization: Bearer abc123";
/// The first bytes of the body too large for one recorded field.
pub(crate) const TRANSCRIPT_BIG_HEAD: &str = "TRANSCRIPT-BIG-HEAD-4a7c";
/// The last bytes of that body.
pub(crate) const TRANSCRIPT_BIG_TAIL: &str = "TRANSCRIPT-BIG-TAIL-9f21";
/// A file body longer than one recorded argument field may keep.
pub(crate) fn transcript_big_body() -> String {
    format!(
        "{TRANSCRIPT_BIG_HEAD}{}{TRANSCRIPT_BIG_TAIL}",
        "x".repeat(8 * 1024)
    )
}

/// The file the overflow run reads in many small ranges. Its results are each
/// larger than one recorded result field, so a scripted job overruns the
/// job-wide transcript budget and must fall back to compact rows.
pub(crate) const OVERFLOW_PATH: &str = "overflow.txt";
/// How many reads the overflow run issues in one turn.
pub(crate) const OVERFLOW_CALLS: usize = 140;
/// The bytes of one line in the overflow file.
pub(crate) const OVERFLOW_LINE_BYTES: usize = 1500;
/// The lines in the overflow file; each call reads two.
pub(crate) const OVERFLOW_LINES: usize = 300;

/// The overflow file: every line distinct, every line the same size.
pub(crate) fn overflow_body() -> String {
    let mut body = String::with_capacity(OVERFLOW_LINES * OVERFLOW_LINE_BYTES);
    for index in 0..OVERFLOW_LINES {
        let prefix = format!("line-{index:04}-");
        body.push_str(&prefix);
        body.push_str(&"x".repeat(OVERFLOW_LINE_BYTES - 1 - prefix.len()));
        body.push('\n');
    }
    body
}

pub(super) fn response(actor: &str, stage: usize, mode: u8) -> String {
    if mode == 104 {
        // One turn issues many ranged reads of a large file. Each result is
        // larger than one recorded result field, so the job-wide transcript
        // budget runs out mid-batch; every later call must still record its
        // compact facts.
        if actor == "parent" {
            return match stage {
                0 => {
                    let calls = (0..OVERFLOW_CALLS)
                        .map(|index| {
                            (
                                "read",
                                json!({
                                    "path":OVERFLOW_PATH,
                                    "start_line":1 + index * 2,
                                    "line_count":2,
                                }),
                            )
                        })
                        .collect();
                    tool_batch("overflow", calls)
                }
                _ => answer("overflow run finished"),
            };
        }
    }
    if mode == 102 {
        // A read-only, approval-free call the run really completes: the
        // journal needs one succeeded call to prove what a success omits.
        // `read` is in the lean default set; `list` is withheld unless the
        // operator names it in `[tools] allow`.
        return if stage == 0 {
            tool("read", json!({"path":"tracked.txt"}), false)
        } else {
            answer("status read")
        };
    }
    if mode == 103 {
        // One helper that writes a file, runs a real command through its own
        // `shell`, then writes a body too large for one recorded field: the
        // durable transcript must carry every call's arguments and result.
        if actor == "parent" {
            return match stage {
                0 => tool(
                    "helpers",
                    json!({
                        "description":"write and run",
                        // The prompt carries the child's id, which is what the
                        // transport routes the child's own requests by.
                        "prompt":"write for child-c1 and run the command",
                        "subagent_type":"general",
                        "agent_id":"child-c1",
                    }),
                    false,
                ),
                _ => answer("transcript parent finished"),
            };
        }
        if actor == "child-c1" {
            return match stage {
                0 => tool(
                    "write",
                    json!({"path":TRANSCRIPT_PATH,"content":format!("{TRANSCRIPT_SECRET_ARGUMENT}\n")}),
                    false,
                ),
                1 => tool(
                    "shell",
                    json!({
                        "command":format!("{TRANSCRIPT_COMMAND} {TRANSCRIPT_STDOUT} {TRANSCRIPT_SECRET_RESULT}"),
                        "description":"run the transcript command",
                    }),
                    false,
                ),
                2 => tool(
                    "write",
                    json!({"path":TRANSCRIPT_PATH,"content":transcript_big_body()}),
                    false,
                ),
                _ => answer("child-c1 wrote and ran"),
            };
        }
    }
    if mode == 101 {
        return if stage == 0 {
            tool(unknown_tool_name().as_str(), json!({"probe": true}), false)
        } else {
            answer("unknown tool run finished")
        };
    }
    if actor == "parent" {
        return match stage {
            // One synchronous `helpers` call: it launches child-c1, waits for
            // it, and returns its summary in the same turn.
            0 => tool(
                "helpers",
                json!({
                    "description":"write diag-a",
                    "prompt":format!("write for child-c1 {ARG_SENTINEL}"),
                    "subagent_type":"general",
                    "agent_id":"child-c1",
                }),
                true,
            ),
            // A second helper reusing a live child's agent id: the task history
            // refuses it as a tool error the model can read, with no child
            // created and no effect.
            1 => tool(
                "helpers",
                json!({
                    "description":"second child",
                    "prompt":"second child",
                    "subagent_type":"general",
                    "agent_id":"child-c1",
                }),
                false,
            ),
            _ => answer("diagnostics parent finished after the refusal"),
        };
    }
    if actor == "child-c1" {
        return match stage {
            0 => tool(
                "write",
                json!({"path":"diag-a.txt","content":RESULT_SENTINEL}),
                false,
            ),
            1 => tool("read", json!({"path":"diag-a.txt"}), false),
            // A path that escapes the shared working checkout: the exact-write
            // admission denies it before any effect; the loop feeds the denial
            // back and the child continues.
            2 => tool(
                "write",
                json!({"path":"../diag-denied.txt","content":"unassigned"}),
                false,
            ),
            _ => answer("child-c1 work complete"),
        };
    }
    answer("unexpected child")
}

/// One provider turn carrying many tool calls at once, each with its own index
/// and id, so the loop executes the whole batch in one turn.
fn tool_batch(prefix: &str, calls: Vec<(&str, Value)>) -> String {
    let calls: Vec<Value> = calls
        .into_iter()
        .enumerate()
        .map(|(index, (name, args))| {
            json!({
                "index":index,
                "id":format!("call-{prefix}-{index}"),
                "type":"function",
                "function":{"name":name,"arguments":args.to_string()},
            })
        })
        .collect();
    let mut body = String::new();
    body.push_str(&chunk(&json!({
        "choices":[{"index":0,"delta":{"tool_calls":calls},"finish_reason":"tool_calls"}]
    })));
    body.push_str(&usage());
    body
}

fn tool(name: &str, args: Value, reasoning: bool) -> String {
    let mut body = String::new();
    if reasoning {
        body.push_str(&chunk(&json!({
            "choices":[{"index":0,"delta":{"reasoning_content":REASONING_SENTINEL},"finish_reason":null}]
        })));
    }
    body.push_str(&chunk(&json!({
        "choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]
    })));
    body.push_str(&usage());
    body
}

fn answer(text: &str) -> String {
    format!(
        "{}{}",
        chunk(&json!({
            "choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]
        })),
        usage()
    )
}

fn usage() -> String {
    format!(
        "{}data: [DONE]\n\n",
        chunk(&json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}}))
    )
}

fn chunk(value: &Value) -> String {
    format!("data: {value}\n\n")
}
