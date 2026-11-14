//! Scripted provider responses for the direct-mode prepare/status lifecycle
//! tests, served through `script_direct`. The root works alone in its working
//! checkout W; every scenario ends with a plain answer.
//!
//! The retired lifecycle calls (`coding_prepare_candidate`, `coding_status`)
//! have no replacement: the owner prepares the candidate at the end of the run,
//! so each turn that prepared or polled is a plain read of the shared
//! checkout, keeping the call id the test reads the result under.
//!
//! Modes (the next free numbers after the direct-task scripts):
//! - 70: the root writes one file in W, then reads (test a).
//! - 71: the root reads, edits W again, reads, reads again (test b).
//! - 72: a root command writes through `shell`, then reads (test c).
//! - 73: a root that changed nothing reads twice (test e).
//! - 74: a read, a write in W, then a second read (test d).
//! - 87: the root writes in W, reads, hands a no-delta piece to a child with
//!   `helpers`, then reads again (the no-delta merge still moves the source
//!   generation).
//! - 88: the root hands a piece to a held child, writes in W, reads while the
//!   child is live, reads, then reads again after the child completed.
//! - 89: the root writes in W and reads; the resumed run reads again (the
//!   receipt is the test's own mutation target).
//! - 90: the root writes, reads, reads, reads, repairs W, reads, reads and
//!   reads again; the resumed run reads the ready receipt.
//! - 110: the root writes and reads, edits W, and reads again.
//! - 64: the root writes and answers, and the RESUMED run edits W and answers
//!   (the further edit moves the generation, so the first receipt is stale).

use super::super::script::render::{answer, tagged_tool, tool};
use serde_json::{Value, json};

fn result(request: &str, id: &str) -> Value {
    let request: Value = serde_json::from_str(request).unwrap();
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["tool_call_id"] == id)
        .and_then(|message| serde_json::from_str(message["content"].as_str()?).ok())
        .unwrap_or(Value::Null)
}

pub(super) fn response(actor: &str, stage: usize, mode: u8, request: &str) -> String {
    if actor != "parent" {
        // Mode 88's child holds the piece it was handed; the other modes have
        // no child at all, or a child that deliberately changes nothing.
        return match (mode, stage) {
            (88, 0) => tool(
                "write",
                json!({"path":"child-note.txt","content":"child wrote\n"}),
            ),
            (88, _) => answer("child change complete"),
            _ => answer("no child writes in this direct prepare script"),
        };
    }
    match mode {
        70 => match stage {
            0 => tagged_tool(
                "only-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            1 => tagged_tool("direct-prepare", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool("direct-replay", "read", json!({"path":"tracked.txt"})),
            _ => answer("the direct root finished after reading"),
        },
        71 => match stage {
            0 => tagged_tool(
                "first-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            1 => tagged_tool("first-prepare", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool(
                "second-write",
                "write",
                json!({"path":"added.txt","content":"FIXED after the edit\n",
                    "expected_digest":result(request, "first-write")["digest"]}),
            ),
            3 => tagged_tool("stale-status", "read", json!({"path":"tracked.txt"})),
            4 => tagged_tool("second-prepare", "read", json!({"path":"tracked.txt"})),
            5 => tagged_tool("ready-status", "read", json!({"path":"tracked.txt"})),
            _ => answer("the direct root finished after re-reading"),
        },
        72 => match stage {
            0 => tagged_tool(
                "ceiling-escape",
                "shell",
                json!({"command":"tee outside.txt","description":"write outside the checkout","sandbox":true}),
            ),
            1 => tagged_tool("outside-prepare", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool("outside-status", "read", json!({"path":"tracked.txt"})),
            _ => answer("the out-of-checkout change was refused at preparation"),
        },
        73 => match stage {
            0 => tagged_tool("empty-prepare", "read", json!({"path":"tracked.txt"})),
            1 => tagged_tool("empty-status", "read", json!({"path":"tracked.txt"})),
            _ => answer("nothing changed, so nothing was prepared"),
        },
        74 => match stage {
            0 => tagged_tool("fresh-status", "read", json!({"path":"tracked.txt"})),
            1 => tagged_tool(
                "only-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            2 => tagged_tool("working-status", "read", json!({"path":"tracked.txt"})),
            _ => answer("the direct root finished after reading"),
        },
        110 => match stage {
            0 => tagged_tool(
                "first-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            1 => tagged_tool("first-prepare", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool(
                "second-write",
                "write",
                json!({"path":"added.txt","content":"FIXED after the edit\n",
                    "expected_digest":result(request, "first-write")["digest"]}),
            ),
            3 => tagged_tool("undeclared-prepare", "read", json!({"path":"tracked.txt"})),
            _ => answer("the root stopped after the undeclared read"),
        },
        // 64: the first run writes W and answers, so the owner prepares a
        // candidate; the resumed run edits W again and answers, so the
        // generation moves and the first receipt does not describe W.
        64 => match stage {
            0 => tagged_tool(
                "first-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            1 => tagged_tool("first-read", "read", json!({"path":"tracked.txt"})),
            2 => answer("the owner prepared the first candidate"),
            3 => tagged_tool("stale-read", "read", json!({"path":"added.txt"})),
            4 => tagged_tool(
                "second-write",
                "write",
                json!({"path":"added.txt","content":"FIXED after the edit\n",
                    "expected_digest":result(request, "stale-read")["digest"]}),
            ),
            5 => tagged_tool("second-read", "read", json!({"path":"tracked.txt"})),
            _ => answer("the owner prepared the second candidate"),
        },
        87 => match stage {
            0 => tagged_tool(
                "empty-write",
                "write",
                json!({"path":"new-a.txt","content":"the root's own delta\n"}),
            ),
            1 => tagged_tool("empty-first-prepare", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool(
                "empty-task",
                "helpers",
                json!({"description":"write new-b","prompt":"write for child-b","subagent_type":"general","agent_id":"child-b"}),
            ),
            3 => tagged_tool("empty-stale-status", "read", json!({"path":"tracked.txt"})),
            4 => tagged_tool(
                "empty-second-prepare",
                "read",
                json!({"path":"tracked.txt"}),
            ),
            _ => answer("the no-delta child still moved the source generation"),
        },
        88 => match stage {
            0 => tagged_tool(
                "call-task",
                "helpers",
                json!({"description":"write child-note.txt","prompt":"write for child-active","subagent_type":"general","agent_id":"child-active"}),
            ),
            1 => tagged_tool("active-read", "read", json!({"path":"tracked.txt"})),
            2 => tagged_tool(
                "active-write",
                "write",
                json!({"path":"tracked.txt","content":"FIXED\n",
                    "expected_digest":result(request, "active-read")["digest"]}),
            ),
            3 => tagged_tool("early-prepare", "read", json!({"path":"tracked.txt"})),
            4 => tagged_tool("early-status", "read", json!({"path":"tracked.txt"})),
            5 => tagged_tool("later-prepare", "read", json!({"path":"tracked.txt"})),
            6 => tagged_tool("replay-prepare", "read", json!({"path":"tracked.txt"})),
            7 => tagged_tool("final-status", "read", json!({"path":"tracked.txt"})),
            _ => answer("candidate prepared after the child completed"),
        },
        89 => match stage {
            0 => tagged_tool("second-read", "read", json!({"path":"tracked.txt"})),
            1 => tagged_tool(
                "second-write",
                "write",
                json!({"path":"tracked.txt","content":"FIXED\n",
                    "expected_digest":result(request, "second-read")["digest"]}),
            ),
            2 => tagged_tool("second-prepare", "read", json!({"path":"tracked.txt"})),
            3 => answer("the prepared candidate remains review-only"),
            4 => tagged_tool("resumed-prepare", "read", json!({"path":"tracked.txt"})),
            _ => answer("the resumed coordinator replayed the receipt"),
        },
        90 => match stage {
            0 => tagged_tool("broken-read", "read", json!({"path":"tracked.txt"})),
            1 => tagged_tool(
                "broken-write",
                "write",
                json!({"path":"tracked.txt","content":"BROKEN\n",
                    "expected_digest":result(request, "broken-read")["digest"]}),
            ),
            2 => tagged_tool("first-prepare", "read", json!({"path":"tracked.txt"})),
            3 => tagged_tool("first-replay", "read", json!({"path":"tracked.txt"})),
            4 => tagged_tool("failed-status", "read", json!({"path":"tracked.txt"})),
            5 => tagged_tool(
                "repair-write",
                "write",
                json!({"path":"tracked.txt","content":"FIXED\n",
                    "expected_digest":result(request, "broken-write")["digest"]}),
            ),
            6 => tagged_tool("second-prepare", "read", json!({"path":"tracked.txt"})),
            7 => tagged_tool("second-replay", "read", json!({"path":"tracked.txt"})),
            8 => tagged_tool("ready-status", "read", json!({"path":"tracked.txt"})),
            9 => answer("the repaired generation prepared a ready candidate"),
            10 => tagged_tool("resumed-prepare", "read", json!({"path":"tracked.txt"})),
            _ => answer("the resumed coordinator replayed the ready receipt"),
        },
        _ => answer("unexpected direct-prepare parent mode"),
    }
}
