//! Scripted provider responses for the direct-mode convergence lifecycle
//! tests (the next free numbers after the root-progress scripts). Every
//! scenario is a root whose calls stop changing the job, so the owner's
//! DIRECT stall bound — not a forced `finish` next action — is what ends it.
//!
//! The retired lifecycle calls (`coding_prepare_candidate`, `coding_status`)
//! and the retired `await_children` have no replacement: the owner prepares at
//! the end of the run and a `helpers` call already waits for its helper, so
//! each such turn is a plain read of the shared checkout.
//!
//! - 91: the root hands a piece to a helper, reads, then repeats a read over and
//!   over (tests a and b: nothing the job records changes).
//! - 92: long runs of read-only polling, each ended by a helper call (test c: a
//!   root that keeps changing the job between polls is never stalled).
//! - 93: the root hands a piece to a child whose request the transport holds,
//!   then reads well past the bound, then reads again (regression a: a live
//!   child is not a stall).
//! - 94: a helper call repeated with no progress, and once more after that
//!   child completed (progress-retry: an identical refused call is admitted only
//!   after real progress).

use serde_json::{Value, json};

use super::super::script::render::{answer, tagged_tool, tool};

/// Root requests a scenario makes before it answers unprompted. A stop must
/// end every scenario that keeps calling long before this many.
const ROOT_CALLS: usize = 120;

/// Read-only polls before the helper call in mode 92, and again after it. Each
/// run stays short of the direct bound while the two together outlast it, so
/// only a bound that forgets to reset on progress can stop the scenario. The
/// poll is a plain file read: a `coding_status` re-verified W's delta, and a
/// hundred of those cost minutes.
const POLL_RUN: usize = 60;

/// The one `helpers` call mode 94 repeats: the same agent, the same piece.
/// Only the tool-call tag differs, so a repeat is byte-identical to admission.
const RETAKE: &str = r#"{"description":"write file-a.txt","prompt":"write for child-b; owned candidate","subagent_type":"general","agent_id":"child-b"}"#;

pub(super) fn response(actor: &str, stage: usize, mode: u8, request: &str) -> String {
    if actor != "parent" {
        return child(stage, mode, actor, request);
    }
    match mode {
        91 => stalled_prepare_root(stage),
        92 => progressing_root(stage),
        93 => held_child_root(stage),
        94 => retaking_root(stage),
        _ => answer("unexpected direct-convergence parent mode"),
    }
}

/// One child completes, so the advisory view suggests a preparation. The root
/// then keeps reading the checkout over and over.
fn stalled_prepare_root(stage: usize) -> String {
    match stage {
        0 => helper("call-task-one", "child-one", "tracked.txt"),
        1 => tagged_tool("status-ready", "read", json!({"path":"tracked.txt"})),
        stage if stage >= ROOT_CALLS => answer("gave up on the candidate"),
        stage => tagged_tool(
            &format!("prepare-{stage}"),
            "read",
            json!({"path":"tracked.txt"}),
        ),
    }
}

/// Long runs of polling, each ended by a helper call.
fn progressing_root(stage: usize) -> String {
    match stage {
        stage if stage < POLL_RUN => read(stage),
        stage if stage == POLL_RUN => helper("call-task-a", "child-a", "new-a.txt"),
        stage if stage <= 2 * POLL_RUN => read(stage),
        _ => answer("the job kept changing between polls"),
    }
}

/// Polls well past the bound while the child's request is held.
fn held_child_root(stage: usize) -> String {
    match stage {
        0 => helper("call-task-active", "child-active", "tracked.txt"),
        1..=20 => status(stage),
        21 => tagged_tool("prepare", "read", json!({"path":"tracked.txt"})),
        22 => tagged_tool("final-status", "read", json!({"path":"tracked.txt"})),
        _ => answer("candidate prepared after the child completed"),
    }
}

/// The same helper call three times: the piece is handed to a child while the
/// first one is live, repeated with no progress, then once more after that child
/// completed.
fn retaking_root(stage: usize) -> String {
    match stage {
        0 => helper("call-task-first", "child-a", "file-a.txt"),
        1 => retake("task-repeat"),
        2 => tagged_tool("status-between", "read", json!({"path":"tracked.txt"})),
        3 => retake("task-no-progress"),
        4 => retake("task-after-progress"),
        _ => answer("the identical helper call was admitted after the child completed"),
    }
}

fn child(stage: usize, mode: u8, actor: &str, request: &str) -> String {
    match mode {
        // The child's merge is what moves W, so the advisory view suggests a
        // preparation once it has joined. `tracked.txt` exists at the baseline,
        // so replacing it needs the digest of a fresh read.
        91 | 93 => match stage {
            0 => tool("read", json!({"path":"tracked.txt"})),
            1 => tagged_tool(
                "write",
                "write",
                json!({"path":"tracked.txt","content":"FIXED\n","expected_digest":read_digest(request)}),
            ),
            _ => answer("child change complete"),
        },
        92 => match stage {
            0 => tagged_tool(
                "write",
                "write",
                json!({"path":"new-a.txt","content":format!("native {actor}\n")}),
            ),
            _ => answer("child work complete"),
        },
        // Mode 94's children change nothing: the helper call itself is the
        // progress, and a delta would make the retaken path root-changed.
        94 => answer("child work complete"),
        _ => answer("no child runs for this direct-convergence mode"),
    }
}

/// One read-only poll of the job, tagged by stage.
fn status(stage: usize) -> String {
    tagged_tool(
        &format!("poll-{stage}"),
        "read",
        json!({"path":"tracked.txt"}),
    )
}

/// One read-only poll that only has to change nothing: a plain file read, so a
/// long run of them stays cheap.
fn read(stage: usize) -> String {
    tagged_tool(
        &format!("poll-{stage}"),
        "read",
        json!({"path":"tracked.txt"}),
    )
}

/// One `helpers` call handing `agent` its piece. The prompt carries
/// `write for <agent>` so the transport attributes the child's turns to it.
fn helper(tag: &str, agent: &str, path: &str) -> String {
    tagged_tool(
        tag,
        "helpers",
        json!({"description":format!("write {path}"),"prompt":format!("write for {agent}; owned candidate"),"subagent_type":"general","agent_id":agent}),
    )
}

/// Mode 94's repeated call: `RETAKE` verbatim, so admission sees the same call.
fn retake(tag: &str) -> String {
    tagged_tool(tag, "helpers", serde_json::from_str(RETAKE).unwrap())
}

/// The digest the child's fresh `read` reported. Replacing a file
/// that exists at the baseline needs it, so the child reads before it writes.
fn read_digest(request: &str) -> Option<String> {
    let body: Value = serde_json::from_str(request).ok()?;
    let read = body["messages"]
        .as_array()?
        .iter()
        .rev()
        .find(|message| message["tool_call_id"] == "call-read")?;
    let result: Value = serde_json::from_str(read["content"].as_str()?).ok()?;
    result["digest"].as_str().map(str::to_owned)
}
