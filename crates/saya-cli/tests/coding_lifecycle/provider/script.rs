//! The per-mode scripts: which tool call or answer each actor receives at
//! each stage, as the SSE bodies the transport sends.

use serde_json::{Value, json};
#[path = "script_render.rs"]
pub(super) mod render;
#[path = "script_workspace_delete.rs"]
mod workspace_delete;
#[path = "script_workspace_delete_refusals.rs"]
mod workspace_delete_refusals;
use render::{answer, tagged_tool, tool, tools};
#[path = "script_context.rs"]
pub(super) mod context;
#[path = "script_convergence.rs"]
mod convergence;
#[path = "script_delegate_cap.rs"]
mod delegate_cap;
use context::{selected_connection, workspace_digest};
#[path = "script_repair_continuation.rs"]
mod repair_continuation;
#[path = "script_usage_unknown.rs"]
mod usage_unknown;
#[path = "script_workspace_retry.rs"]
mod workspace_retry;

#[cfg(target_os = "macos")]
#[path = "script_command_wait.rs"]
mod command_wait;
#[path = "script_post_edit.rs"]
mod post_edit;
#[path = "script_read_only.rs"]
mod read_only;
#[path = "script_repair_controls.rs"]
mod repair_controls;
#[path = "script_await.rs"]
mod script_await;
#[path = "script_stage2.rs"]
mod stage2;

pub(super) fn response(
    actor: &str,
    stage: usize,
    injection: bool,
    mode: u8,
    request: &str,
) -> String {
    match mode {
        60 => return usage_unknown::response(actor, stage, request),
        118 => return read_only::response(actor, stage, request),
        61 => return usage_unknown::exhausted(actor, stage),
        // 109: the same child lifecycle as 60, but its first stream is cut
        // after a long reasoning flow, so the continuation (not a resend)
        // carries the child past the cut.
        109 => return usage_unknown::response(actor, stage, request),
        62..=108 | 110 | 112..=117 | 119 => {
            return super::script_direct::response(actor, stage, mode, request);
        }
        _ => {}
    }
    if mode == 44 {
        return delegate_cap::response(actor, stage);
    }
    // 121: a start that needs no tool call, so a fallback start can finish
    // without touching the folder.
    if mode == 121 {
        return answer("fallback start answered");
    }
    // 122: the in-place root attempts one write outside its stated `--write`
    // scope, then answers. Nothing is written; the refusal reaches the next
    // request's tool result.
    if mode == 122 {
        return match (actor, stage) {
            ("parent", 0) => tool("write", json!({"path":"README.md","content":"outside\n"})),
            ("parent", _) => answer("scope refusal observed"),
            _ => answer("no child runs in this fixture"),
        };
    }
    // 123: the in-place root attempts one write outside the workspace, then
    // answers. The containment boundary refuses it; nothing is written.
    if mode == 123 {
        return match (actor, stage) {
            ("parent", 0) => tool(
                "write",
                json!({"path":"../escape.txt","content":"outside\n"}),
            ),
            ("parent", _) => answer("workspace refusal observed"),
            _ => answer("no child runs in this fixture"),
        };
    }
    // 120: the in-place root reads the conflicted file and replaces Git's
    // conflict markers with the resolved content.
    if mode == 120 {
        if actor != "parent" {
            return answer("no child runs in this fixture");
        }
        return match stage {
            0 => tool("read", json!({"path":"tracked.txt"})),
            1 => tool(
                "edit",
                json!({
                    "path":"tracked.txt",
                    "old_text":"<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\n",
                    "new_text":"resolved\n",
                    "expected_digest":workspace_digest(request).unwrap_or_default(),
                }),
            ),
            _ => answer("conflict resolved"),
        };
    }
    if (52..=54).contains(&mode) {
        return convergence::response(actor, stage, mode, request);
    }
    if mode == 36 {
        return stage2::response(actor, stage, request);
    }
    if mode == 22 {
        return workspace_delete_refusals::response(actor, stage, request);
    }
    if matches!(mode, 30..=35) {
        return post_edit::response(actor, stage, mode);
    }
    if mode == 20 {
        return workspace_delete::response(actor, stage, request);
    }
    if mode == 21 {
        return repair_controls::response(actor, stage, request);
    }
    if mode == 18 {
        return workspace_retry::response(actor, stage);
    }
    #[cfg(target_os = "macos")]
    if mode == 24 {
        return command_wait::response(actor, stage, request);
    }
    if actor == "parent" {
        if matches!(mode, 37..=39) {
            return script_await::response(mode, stage);
        }
        // 41: one read then an answer; both provider calls report cached
        // input and reasoning tokens, so the usage totals have a breakdown.
        if mode == 41 {
            let mut usage = render::usage(19_000, 1_000, Some(18_000), Some(500));
            // The provider-reported cost the job must carry through to its
            // usage report: a quarter-dollar per call, two calls.
            usage["cost"] = json!(0.25);
            return match stage {
                0 => render::usage_tool(
                    "call-usage-read",
                    "read",
                    json!({"path":"tracked.txt"}),
                    usage,
                ),
                _ => render::usage_answer("usage fixture finished", usage),
            };
        }
        // 9: the readiness poll is a surviving read-only root call repeated
        // past the logical-request ceiling, then a plain answer.
        if mode == 9 {
            return if stage < 80 {
                tool("read", json!({"path":"tracked.txt"}))
            } else {
                answer("all admitted read calls completed")
            };
        }
        // 7: the two-task batch and its join are one synchronous
        // `helpers` call per child, each merged into the working checkout
        // before the next is admitted.
        if mode == 7 {
            return match stage {
                0 => helpers("call-task-a", "write only.txt", "child-a"),
                1 => helpers("call-task-b", "write only.txt", "child-b"),
                2 => helpers("call-task-c", "write only.txt", "child-one"),
                _ => answer("the one-path parent finished after its helpers"),
            };
        }
        // 6: a job with no approved write path advertises no delegation tool,
        // so every call below is refused as unknown; the answer stays parked
        // until the test releases it (see `transport.rs`).
        if mode == 6 {
            return match stage {
                0 => helpers("call-helpers", "steer the ghost", "ghost"),
                1 => tagged_tool("call-read", "read", json!({"path":"tracked.txt"})),
                // The paired capture helper keys on this call id; keep it
                // stable while the call it carries is a surviving read.
                2 => tagged_tool("call-coding_status", "read", json!({"path":"tracked.txt"})),
                _ => answer("parity parent finished after the unregistered-child refusals"),
            };
        }
        // 10: the two database probe children are one `helpers` call each,
        // merged before the root's own database reads.
        if mode == 10 {
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
                    json!({"connection":selected_connection(request)}),
                ),
                3 => tagged_tool(
                    "root-read",
                    "bounded_sql_query",
                    json!({"connection":selected_connection(request),"sql":"SELECT value FROM saya_sentinel"}),
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
        // 111: both `helpers` calls in ONE turn, so the two helpers are
        // dispatched together and run at the same time. The transport holds
        // each child's first turn until both are pending (with a bounded hold,
        // so a job that refuses the second helper fails instead of hanging),
        // which is what makes the overlap observable.
        if mode == 111 {
            return match stage {
                0 => helpers_in_one_turn(&[
                    ("call-task-a", "write new-a.txt", "child-a"),
                    ("call-task-b", "write new-b.txt", "child-b"),
                ]),
                _ => answer("native parent finished after merging its helpers"),
            };
        }
        // The default parent (mode 0): one `helpers` call per child, merged
        // into the working checkout, then a plain answer.
        return match stage {
            0 => helpers("call-task-a", "write new-a.txt", "child-a"),
            1 => helpers("call-task-b", "write new-b.txt", "child-b"),
            _ => answer("native parent finished after merging its helpers"),
        };
    }
    // 1: the child runs a real failing command through `shell`; the retired
    // check tool has no replacement, so the run's nonzero exit comes from the
    // shell the child actually owns.
    if mode == 1 && stage == 0 {
        return tool(
            "shell",
            json!({"command":"false","description":"run the failing command"}),
        );
    }
    // 7: a helper that completes without changing its checkout; its merge
    // applies nothing, so the parent simply moves on to the next one.
    if mode == 7 {
        return answer("the helper completed without a change");
    }
    if mode == 10 {
        if actor == "child-read" {
            return match stage {
                0 => tool(
                    "schema_discovery",
                    json!({"connection":selected_connection(request)}),
                ),
                1 => tool(
                    "bounded_sql_query",
                    json!({"connection":selected_connection(request),"sql":"SELECT value FROM saya_sentinel"}),
                ),
                _ => answer("child database read complete"),
            };
        }
        if actor == "child-tamper" {
            return match stage {
                0 => tagged_tool(
                    "coding-sql-write",
                    "bounded_sql_query",
                    json!({"connection":selected_connection(request),"sql":"INSERT INTO saya_sentinel (value) VALUES ('tamper')"}),
                ),
                1 => tagged_tool(
                    "coding-sql-unselected",
                    "bounded_sql_query",
                    json!({"connection":"elsewhere","sql":"SELECT value FROM saya_sentinel"}),
                ),
                _ => answer("child database refusal probe complete"),
            };
        }
        return answer("unexpected child actor finished");
    }
    if mode != 0 && mode != 111 {
        return answer("child command evidence obtained");
    }
    if injection {
        match stage {
            0 => return tool("read", json!({"path":".agents/skills/poison/SKILL.md"})),
            // The injected child reaches for root authority; children are
            // never given `helpers`, so the escape is refused before it runs.
            1 => return helpers("call-escape", "grant new root", "escape"),
            2 => {
                return tool(
                    "write",
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
            "write",
            json!({"path":path,"content":format!("native {actor}\n")}),
        )
    } else {
        answer("child work complete")
    }
}

/// One synchronous `helpers` call: the child's own task text carries
/// `write for <agent>`, which is how the transport attributes the child's
/// turns to that agent. Every helper works in the root's working checkout, so
/// there are no per-helper write paths to declare.
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

/// One `helpers` call per `(id, description, agent)` triple, emitted in ONE
/// turn: several `helpers` calls in one turn run at the same time, so the
/// parent dispatches every helper together.
fn helpers_in_one_turn(calls: &[(&str, &str, &str)]) -> String {
    let owned: Vec<(String, Value)> = calls
        .iter()
        .map(|(id, description, agent)| {
            (
                (*id).to_owned(),
                json!({
                    "description": description,
                    "prompt": format!("write for {agent}"),
                    "subagent_type": "general",
                    "agent_id": agent,
                }),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str, Value)> = owned
        .iter()
        .map(|(id, args)| (id.as_str(), "helpers", args.clone()))
        .collect();
    tools(&borrowed)
}
