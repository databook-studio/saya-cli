//! Scripted provider responses for the direct-mode finish lifecycle tests: the
//! root loop ends and the OWNER prepares the candidate. Served through
//! `script_direct`. Every scenario ends with a plain answer, and none of them
//! calls a lifecycle tool — preparing is the owner's job at the end of the run.
//!
//! Modes (the next free numbers after the direct-prepare scripts):
//! - 75: the root writes one file in W and answers (test a: the owner prepares
//!   the candidate from W and the receipt says ready).
//! - 76: the root changes nothing and answers (test b).
//! - 77: a root command writes through `shell`, then answers (test c: the
//!   refused preparation is not a root failure).
//! - 78: the root writes one file, then reads the checkout, then answers (test
//!   d: the advisory read, never a delegate-only finish instruction).
//! - 79: the root writes one file, hands one piece to a helper, then answers
//!   (test e: preparation is skipped while children are live).
//! - 80: the root writes a file, then answers (test c: the owner prepares and
//!   the receipt says not ready).

use super::super::script::render::{answer, tagged_tool};
use serde_json::json;

pub(super) fn response(actor: &str, stage: usize, mode: u8, _request: &str) -> String {
    if actor != "parent" {
        return match (mode, stage) {
            (79, 0) => tagged_tool(
                "child-write",
                "write",
                json!({"path":"child-file.txt","content":"child wrote\n"}),
            ),
            (79, _) => answer("the helper child completed its piece"),
            _ => answer("no child runs in the direct finish scripts"),
        };
    }
    match mode {
        75 => match stage {
            0 => tagged_tool(
                "only-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            _ => answer("the direct root finished without preparing"),
        },
        76 => answer("the direct root changed nothing"),
        77 => match stage {
            0 => tagged_tool(
                "ceiling-escape",
                "shell",
                // `tee` with no stdin creates an empty file in the working
                // checkout: a command side effect.
                json!({"command":"tee outside.txt","description":"write outside the checkout","sandbox":true}),
            ),
            _ => answer("the direct root finished after an out-of-checkout command"),
        },
        78 => match stage {
            0 => tagged_tool(
                "only-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            // The retired `await_children` has no replacement: a `helpers` call
            // always names one helper, so the empty await is a plain read.
            1 => tagged_tool("await-advisory", "read", json!({"path":"tracked.txt"})),
            _ => answer("the direct root finished after reading with no children"),
        },
        79 => match stage {
            0 => tagged_tool(
                "root-write",
                "write",
                json!({"path":"root-edit.txt","content":"root wrote\n"}),
            ),
            1 => tagged_tool(
                "delegate-live",
                "helpers",
                json!({"description":"write child-file.txt","prompt":"write for child-a","subagent_type":"general","agent_id":"child-a"}),
            ),
            _ => answer("the direct root finished with a child it never awaited"),
        },
        80 => match stage {
            0 => tagged_tool(
                "only-write",
                "write",
                json!({"path":"added.txt","content":"PLAIN\n"}),
            ),
            _ => answer("the direct root finished with a change in the working checkout"),
        },
        _ => answer("unexpected direct-finish parent mode"),
    }
}
