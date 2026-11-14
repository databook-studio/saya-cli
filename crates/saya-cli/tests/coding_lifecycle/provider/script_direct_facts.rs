//! Scripted provider responses for the direct-mode readiness facts: the changed
//! paths and the child failures the advisory readiness carries.
//!
//! Modes (the next free numbers after the readiness-cache scripts):
//! - 101: the root modifies one baseline file, creates one, deletes one, then
//!   reads the checkout twice (tests a and f: the per-path change list with its
//!   kind, and a cache hit that returns exactly the fresh computation).
//! - 102: the root creates 65 files, then reads (test b: the list is bounded to
//!   64 and says so).
//! - 103: the root writes one file, then answers (test c: the owner prepares and
//!   the readiness carries the change).
//! - 104: the root hands one piece to a helper whose streams are cut until its
//!   requests exhaust, then reads (test d: the child failure with its class).
//! - 105: the root reads, hands one piece to a helper, reads again, then answers
//!   (test e: the child state alone invalidates the cached facts; the transport
//!   holds the second read until the test has seen the child fail).
//! - 106: the root modifies one baseline file, then reads twice (test f: a hit
//!   returns exactly the fresh computation, new facts included).
//! - 113: the root writes one file in one turn and another in the next, then
//!   answers (the operator-rewind test: two checkpointed turns, so a rewind to
//!   the first restores the checkout to exactly that state).
//! - 114: the same two turns, then the model lists the checkpoints, restores
//!   the first for one path, reads both files, and restores the first for every
//!   path (the model-facing rewind tool).

use super::super::script::render::{answer, tagged_tool};
use serde_json::{Value, json};

/// Paths the truncation scenario creates: one more than the readiness bound.
const CREATED_PATHS: usize = 65;

pub(super) fn response(actor: &str, stage: usize, mode: u8, request: &str) -> String {
    if actor != "parent" {
        return child(stage, mode);
    }
    match mode {
        101 => match stage {
            // Replacing an existing file needs the digest a fresh read
            // observed; creating one does not.
            0 => tagged_tool("read-tracked", "read", json!({"path":"tracked.txt"})),
            1 => tagged_tool(
                "write-modified",
                "write",
                json!({"path":"tracked.txt","content":"changed\n","expected_digest":observed_digest(request, "read-tracked")}),
            ),
            2 => tagged_tool(
                "write-created",
                "write",
                json!({"path":"added.txt","content":"added\n"}),
            ),
            3 => tagged_tool("read-obsolete", "read", json!({"path":"obsolete.txt"})),
            4 => tagged_tool(
                "delete-obsolete",
                "delete",
                json!({"path":"obsolete.txt","expected_digest":observed_digest(request, "read-obsolete")}),
            ),
            5 => status(0),
            6 => status(1),
            _ => answer("the root changed three paths and read the advisory readiness twice"),
        },
        102 => {
            if stage < CREATED_PATHS {
                return tagged_tool(
                    &format!("write-{stage}"),
                    "write",
                    json!({"path":format!("f{stage:02}.txt"),"content":format!("{stage}\n")}),
                );
            }
            match stage {
                stage if stage == CREATED_PATHS => status(0),
                _ => answer("the root created more paths than the readiness lists"),
            }
        }
        103 => match stage {
            0 => tagged_tool(
                "write-failing",
                "write",
                json!({"path":"added.txt","content":"PLAIN\n"}),
            ),
            _ => answer("the root left a change in the working checkout"),
        },
        106 => match stage {
            0 => tagged_tool("read-tracked", "read", json!({"path":"tracked.txt"})),
            1 => tagged_tool(
                "write-modified",
                "write",
                json!({"path":"tracked.txt","content":"changed\n","expected_digest":observed_digest(request, "read-tracked")}),
            ),
            2 => status(0),
            3 => status(1),
            _ => answer("the root changed one path and read the advisory readiness twice"),
        },
        113 => match stage {
            0 => tagged_tool(
                "write-turn-one",
                "write",
                json!({"path":"turn-one.txt","content":"turn one\n"}),
            ),
            1 => tagged_tool(
                "write-turn-two",
                "write",
                json!({"path":"turn-two.txt","content":"turn two\n"}),
            ),
            _ => answer("the root changed one path in each of two turns"),
        },
        // 114: the model-facing rewind. Two turns change the checkout, the
        // model lists the checkpoints, restores the first one for one path,
        // reads both files, then restores the first one for every path.
        114 => match stage {
            0 => tagged_tool(
                "write-one",
                "write",
                json!({"path":"turn-one.txt","content":"one\n"}),
            ),
            1 => tagged_tool("read-one", "read", json!({"path":"turn-one.txt"})),
            2 => tagged_tool(
                "write-one-changed",
                "write",
                json!({"path":"turn-one.txt","content":"changed\n","expected_digest":observed_digest(request, "read-one")}),
            ),
            3 => tagged_tool(
                "write-two",
                "write",
                json!({"path":"turn-two.txt","content":"two\n"}),
            ),
            4 => tagged_tool("rewind-list", "rewind", json!({})),
            5 => tagged_tool(
                "rewind-one",
                "rewind",
                json!({"checkpoint":observed_checkpoint(request, "rewind-list"),"paths":["turn-one.txt"]}),
            ),
            6 => tagged_tool("read-after-rewind", "read", json!({"path":"turn-one.txt"})),
            7 => tagged_tool("read-turn-two", "read", json!({"path":"turn-two.txt"})),
            8 => tagged_tool(
                "rewind-all",
                "rewind",
                json!({"checkpoint":observed_checkpoint(request, "rewind-list")}),
            ),
            _ => answer("the root rewound the checkout and kept the later file"),
        },
        104 => match stage {
            0 => tagged_tool(
                "call-task",
                "helpers",
                json!({"description":"write child-one","prompt":"write for child-one","subagent_type":"general","agent_id":"child-one"}),
            ),
            1 => status(0),
            _ => answer("the root read the readiness after a failed child"),
        },
        105 => match stage {
            0 => status(0),
            1 => tagged_tool(
                "delegate-failing",
                "helpers",
                json!({"description":"write child-one","prompt":"write for child-one","subagent_type":"general","agent_id":"child-one"}),
            ),
            2 => status(1),
            _ => answer("the root read the readiness after a child failed"),
        },
        _ => answer("unexpected direct-facts parent mode"),
    }
}

/// The child's scripted turns. Modes 104 and 105 cut this child's streams until
/// its requests exhaust, so these exist only so a non-cut turn would still be
/// well-formed.
fn child(stage: usize, mode: u8) -> String {
    match mode {
        104 | 105 => match stage {
            0 => tagged_tool(
                "child-write",
                "write",
                json!({"path":"child-one.txt","content":"child-one wrote\n"}),
            ),
            _ => answer("child-one completed its piece"),
        },
        _ => answer("no child runs for this direct-facts mode"),
    }
}

/// One read of the shared checkout, tagged by stage so the test can read each
/// result. The retired `coding_status` poll has no replacement: the owner's
/// advisory readiness is not a model tool.
fn status(stage: usize) -> String {
    tagged_tool(
        &format!("call-status-{stage}"),
        "read",
        json!({"path":"tracked.txt"}),
    )
}

/// The digest a previous owner read observed, taken from the request's own
/// transcript rather than a fixture constant.
fn observed_digest(request: &str, id: &str) -> Option<String> {
    let request: Value = serde_json::from_str(request).ok()?;
    let message = request["messages"]
        .as_array()?
        .iter()
        .rev()
        .find(|message| message["tool_call_id"] == id)?;
    let result: Value = serde_json::from_str(message["content"].as_str()?).ok()?;
    result["digest"].as_str().map(str::to_owned)
}

/// The oldest checkpoint id a previous listing observed: the one the scripted
/// root rewinds to, taken from the request's own transcript rather than a
/// fixture constant.
fn observed_checkpoint(request: &str, id: &str) -> Option<String> {
    let request: Value = serde_json::from_str(request).ok()?;
    let message = request["messages"]
        .as_array()?
        .iter()
        .rev()
        .find(|message| message["tool_call_id"] == id)?;
    let result: Value = serde_json::from_str(message["content"].as_str()?).ok()?;
    result["checkpoints"]
        .as_array()?
        .last()?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}
