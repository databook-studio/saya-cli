//! Scripted provider responses for the direct-mode `helpers` tool lifecycle tests.
//!
//! Modes served here (the next free numbers after the existing scripts):
//! - 62: the root hands one piece to a child via `helpers`, then writes another
//!   file itself and finishes (tests a, b, and the completed-merge arm of f).
//! - 63: a `helpers` call names two children over the SAME path; admission refuses
//!   the second before any child exists (test c).
//! - 65: a `helpers` child whose model stream is cut until its requests exhaust,
//!   so it fails; the summary carries its failure class (test d, and the
//!   failed-lease arm of f).
//! - 68: a root with no approved write path calls `helpers`; without a working
//!   checkout it is not advertised, so the loop refuses it as an unknown tool
//!   (test g).
//! - 69: a direct-mode root hands two disjoint pieces to two helpers, one
//!   synchronous `helpers` call each, and answers (test e: each finished child
//!   is merged into the working checkout exactly once).
//! - 64, 70..=74, 87..=90 and 110: the direct-mode prepare/status lifecycle;
//!   their scripts live in `script_direct_prepare`.
//! - 75..=80: the direct-mode finish lifecycle; their scripts live in
//!   `script_direct_finish`.
//! - 66, 67 and 81..=86: the root-workspace-progress lifecycle; their scripts
//!   live in `script_direct_progress`.
//! - 91..=94: the convergence lifecycle; their scripts live in
//!   `script_direct_convergence`.
//! - 95..=100: the advisory readiness cache; their scripts live in
//!   `script_direct_status`.
//! - 101..=106: the advisory readiness facts; their scripts live in
//!   `script_direct_facts`.
//! - 113: two turns that each write one file, so the operator rewind has two
//!   checkpoints; its script lives in `script_direct_facts`.
//! - 114: the same two turns, then the model-facing `rewind` lists the
//!   checkpoints, restores one path, reads both, and restores every path; its
//!   script lives in `script_direct_facts`.
//! - 107: a `helpers` child that completes without changing anything, so its merge
//!   applies nothing and the child stays unmerged (the operator follow-up of an
//!   unmerged child).
//! - 108: the same unmerged child, whose operator follow-up then writes its
//!   assigned path, so the follow-up's own completion merges into W.
//! - 112: one `checklist` write, then reads that change nothing; their script
//!   lives in `script_direct_checklist`.
//! - 115..=117: the progress guards; their scripts live in
//!   `script_direct_progress`.
//! - 119: an in-place root edits `a.txt` twice, creates `new.txt`, and
//!   deletes `old.txt`, so the before-image store records each first touch.

#[path = "script_direct_checklist.rs"]
mod checklist;

#[path = "script_direct_convergence.rs"]
mod convergence;

#[path = "script_direct_prepare.rs"]
mod prepare;

#[path = "script_direct_facts.rs"]
mod facts;

#[path = "script_direct_finish.rs"]
mod finish;

#[path = "script_direct_progress.rs"]
mod progress;

#[path = "script_direct_status.rs"]
mod status;

use serde_json::json;

use super::script::context::workspace_digest;
use super::script::render::{answer, tagged_tool, tool};

pub(super) fn response(actor: &str, stage: usize, mode: u8, request: &str) -> String {
    if mode == 112 {
        return checklist::response(actor, stage, request);
    }
    if (95..=100).contains(&mode) {
        return status::response(actor, stage, mode, request);
    }
    if (101..=106).contains(&mode) || mode == 113 || mode == 114 {
        return facts::response(actor, stage, mode, request);
    }
    if (70..=74).contains(&mode) || (87..=90).contains(&mode) || mode == 110 || mode == 64 {
        return prepare::response(actor, stage, mode, request);
    }
    if (75..=80).contains(&mode) {
        return finish::response(actor, stage, mode, request);
    }
    if (81..=86).contains(&mode) || (66..=67).contains(&mode) || (115..=117).contains(&mode) {
        return progress::response(actor, stage, mode, request);
    }
    if (91..=94).contains(&mode) {
        return convergence::response(actor, stage, mode, request);
    }
    if actor == "parent" {
        return parent(stage, mode, request);
    }
    child(stage, mode, actor, request)
}

fn parent(stage: usize, mode: u8, request: &str) -> String {
    match mode {
        119 => match stage {
            0 => tagged_tool("call-read", "read", json!({"path":"a.txt"})),
            1 => tool(
                "edit",
                json!({
                    "path":"a.txt",
                    "old_text":"alpha",
                    "new_text":"beta",
                    "expected_digest":workspace_digest(request).unwrap_or_default(),
                }),
            ),
            2 => tagged_tool("call-read", "read", json!({"path":"a.txt"})),
            3 => tool(
                "edit",
                json!({
                    "path":"a.txt",
                    "old_text":"beta",
                    "new_text":"gamma",
                    "expected_digest":workspace_digest(request).unwrap_or_default(),
                }),
            ),
            4 => tool("write", json!({"path":"new.txt","content":"fresh\n"})),
            5 => tagged_tool("call-read", "read", json!({"path":"old.txt"})),
            6 => tool(
                "delete",
                json!({
                    "path":"old.txt",
                    "expected_digest":workspace_digest(request).unwrap_or_default(),
                }),
            ),
            _ => answer("the in-place folder was edited, created and deleted"),
        },
        62 => match stage {
            0 => tagged_tool(
                "call-task",
                "helpers",
                json!({"description":"write file-a","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            1 => tool(
                "write",
                json!({"path":"file-b.txt","content":"root wrote after the merge\n"}),
            ),
            _ => answer("the direct root finished after delegating and merging"),
        },
        63 => match stage {
            0 => tagged_tool(
                "call-task",
                "helpers",
                json!({"description":"outside ceiling","prompt":"write outside the ceiling","subagent_type":"general","agent_id":"child-a"}),
            ),
            _ => answer("the overlapping task was refused before any child existed"),
        },
        65 => match stage {
            0 => tagged_tool(
                "call-task",
                "helpers",
                json!({"description":"write file-d","prompt":"write for child-one","subagent_type":"general","agent_id":"child-one"}),
            ),
            _ => answer("the failed child was summarized without a merge"),
        },
        68 => match stage {
            0 => tagged_tool(
                "call-task",
                "helpers",
                json!({"description":"write file-a","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            _ => answer("the task tool was refused for a job with no working checkout"),
        },
        69 => match stage {
            0 => tagged_tool(
                "call-task-a",
                "helpers",
                json!({"description":"write new-a.txt","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            1 => tagged_tool(
                "call-task-b",
                "helpers",
                json!({"description":"write new-b.txt","prompt":"write for child-b","subagent_type":"general","agent_id":"child-b"}),
            ),
            _ => answer("the direct root finished after merging its helpers"),
        },
        // The child completes without changing its checkout: its merge applies
        // nothing, so it stays unmerged.
        107 => match stage {
            0 => tool(
                "helpers",
                json!({"description":"inspect tracked","prompt":"inspect for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            _ => answer("the child's merge applied nothing, so it was not merged"),
        },
        // 108: the same child, whose follow-up invocation (stage 2, below)
        // writes the path it was assigned, so the follow-up's completion is
        // the child's first mergeable one.
        108 => match stage {
            0 => tool(
                "helpers",
                json!({"description":"write tracked","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            _ => answer("the child's merge applied nothing, so it was not merged"),
        },
        _ => answer("unexpected direct-task parent mode"),
    }
}

fn child(stage: usize, mode: u8, actor: &str, request: &str) -> String {
    match mode {
        62 => match stage {
            0 => tool(
                "write",
                json!({"path":"file-a.txt","content":"child-a wrote\n"}),
            ),
            _ => answer("child-a completed its piece"),
        },
        65 => match stage {
            0 => tool(
                "write",
                json!({"path":"file-d.txt","content":"child-one wrote\n"}),
            ),
            _ => answer("child-one completed its piece"),
        },
        // Mode 107: `child-a` inspects its path and changes nothing.
        107 => match stage {
            0 => tool("read", json!({"path":"tracked.txt"})),
            _ => answer("child-a completed its piece without a change"),
        },
        // Mode 108: the child's first invocation changes nothing (stages 0-1);
        // its operator follow-up is the same actor's next turns: it reads its
        // path, then rewrites it with the digest that read returned (stages 2-4).
        108 => match stage {
            0 => tool("read", json!({"path":"tracked.txt"})),
            1 => answer("child-a completed its piece without a change"),
            2 => tool("read", json!({"path":"tracked.txt"})),
            3 => tool(
                "write",
                json!({"path":"tracked.txt","content":"followed up\n","expected_digest":workspace_digest(request)}),
            ),
            _ => answer("the follow-up wrote its assigned path"),
        },
        // Mode 65: the transport cuts this child's streams until its requests
        // exhaust, so this response is never reached; it exists only so a
        // non-cut turn would still be well-formed.
        69 => match stage {
            0 => tool(
                "write",
                json!({"path":if actor == "child-a" { "new-a.txt" } else { "new-b.txt" },"content":format!("native {actor}\n")}),
            ),
            _ => answer("child work complete"),
        },
        _ => answer("no child runs for this direct-task mode"),
    }
}
