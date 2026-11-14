//! Scripted provider responses for the direct-mode advisory readiness cache.
//!
//! Modes (the next free numbers after the direct convergence scripts):
//! - 95: eight reads with nothing changing, then an answer (test a: N reads
//!   compute W's facts once).
//! - 96: two reads, one successful root write, one more read (test b: a
//!   workspace write invalidates the cached facts).
//! - 97: two reads, a `shell` the job runs, one more read (tests c and e: a
//!   command ATTEMPT invalidates too, and the facts it recomputes are the ones
//!   the cache would have returned).
//! - 98: two reads, a `helpers` child that changes no file, one more read (test
//!   d: the merge attempt alone moves the generation).
//! - 99: two reads, then another read (test f: the owner's preparation at the
//!   end of the run recomputes from the checkout).
//! - 100: two reads, then an answer (test f: the owner's preparation at the end
//!   of the run recomputes from the checkout).
//!
//! Modes 99 and 100 hold the parent's third response until the test releases
//! it, so the test can write into the working checkout between the cached reads
//! and the call under test.

use super::super::script::render::{answer, tagged_tool};
use serde_json::json;

/// Reads in mode 95: every one after the first is a hit.
const READS: usize = 8;

pub(super) fn response(actor: &str, stage: usize, mode: u8, _request: &str) -> String {
    if actor != "parent" {
        // Mode 98's child completes without writing anything: its merge still
        // records an attempt, which is the whole point of the test.
        return answer("child-b changed nothing");
    }
    match mode {
        95 => match stage {
            stage if stage < READS => status(stage),
            _ => answer("the root read the checkout without changing anything"),
        },
        96 => match stage {
            0 | 1 => status(stage),
            2 => tagged_tool(
                "call-root-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            3 => status(stage),
            _ => answer("the root wrote once between reads"),
        },
        97 => match stage {
            0 | 1 => status(stage),
            2 => tagged_tool(
                "call-refused-command",
                "shell",
                json!({"command":"true","description":"attempt a command","sandbox":true}),
            ),
            3 => status(stage),
            _ => answer("the command attempt bought no change"),
        },
        98 => match stage {
            0 | 1 => status(stage),
            2 => tagged_tool(
                "call-task",
                "helpers",
                json!({"description":"write child-b","prompt":"write for child-b","subagent_type":"general","agent_id":"child-b"}),
            ),
            3 => status(stage),
            _ => answer("the no-delta child was merged into the working checkout"),
        },
        99 => match stage {
            0 | 1 => status(stage),
            2 => tagged_tool("call-prepare", "read", json!({"path":"tracked.txt"})),
            _ => answer("the root read after an out-of-band change"),
        },
        100 => match stage {
            0 | 1 => status(stage),
            _ => answer("the root left the preparation to the owner"),
        },
        _ => answer("unexpected direct-status mode"),
    }
}

/// One read of the shared checkout, tagged by stage so the test can read each
/// result. The retired `coding_status` poll has no replacement.
fn status(stage: usize) -> String {
    tagged_tool(
        &format!("call-status-{stage}"),
        "read",
        json!({"path":"tracked.txt"}),
    )
}
