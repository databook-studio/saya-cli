//! Scripted provider responses for the root-workspace-progress lifecycle tests
//! (the next free numbers after the direct finish scripts).
//!
//! - 81: one successful root write between two long runs of read-only calls
//!   (test a: a successful write resets the no-progress streak).
//! - 82: 95 read-only calls, then a write outside the checkout, then one more
//!   read (test b: a refused write does not end a run whose other calls
//!   succeed).
//! - 83: 45 successful root writes, then an answer (test c: a root that keeps
//!   writing is not stalled).
//! - 84: 120 successful read-only calls, then an answer (test d: a run of
//!   successful calls that change nothing is never stalled).
//! - 85: 95 read-only calls, then an answer (test d, second arm: a short run of
//!   reads is fine).
//! - 86: reads only, forever (test e: a long run of successful calls is the
//!   stall).
//! - 66: 96 root writes the write authority refuses, then an answer (the
//!   direct stall bound counts only failed or refused calls).
//! - 67: 95 read-only calls, one refused root write, one more read, then an
//!   answer (a refused call between successful ones resets nothing).
//! - 115: four identical reads of one path, then an answer (the repeat guard
//!   notes the third and stops the run on the fourth).
//! - 116: one successful write, then eight failed reads of distinct missing
//!   paths, then an answer (the no-progress guard stops the run).
//! - 117: 120 successful `echo` commands, then an answer (a long productive
//!   run is not stopped).

use serde_json::json;

use super::super::script::render::{answer, tagged_tool};

/// Read-only calls before the one successful write in mode 81.
const READS_BEFORE: usize = 40;
/// Read-only calls after it. 57 leaves the streak at 57: past the delegate-only
/// bound of 36 and short of the direct bound of 96.
const READS_AFTER: usize = 57;
/// Read-only calls before the refused write in mode 82.
const READS_BEFORE_REFUSAL: usize = 95;
/// Successful root writes in mode 83.
const WRITES: usize = 45;
/// Successful read-only calls in mode 84: well past the 96-call bound, which a
/// run of successful calls never reaches.
const SUCCESSFUL_READS: usize = 120;
/// Read-only calls in mode 85: a short run that stays clear of the bound.
const TOLERATED_READS: usize = 95;
/// Read-only calls in mode 86.
const REFUSED_CALLS: usize = 96;
/// Identical reads in mode 115: the third notes the repeat, the fourth stops.
const REPEATED_READS: usize = 4;
/// Failed reads after one write in mode 116: the eighth is the eighth turn
/// without progress.
const NO_PROGRESS_READS: usize = 8;
/// Successful distinct commands in mode 117.
const PRODUCTIVE_COMMANDS: usize = 120;
/// A digest no file can have, so mode 67's guarded replace is refused.
const STALE_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub(super) fn response(actor: &str, stage: usize, mode: u8, _request: &str) -> String {
    let _ = actor;
    match mode {
        81 => match stage {
            stage if stage < READS_BEFORE => read(stage),
            READS_BEFORE => tagged_tool(
                "call-root-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            stage if stage < READS_BEFORE + 1 + READS_AFTER => read(stage),
            _ => answer("the root wrote once and kept reading"),
        },
        82 => match stage {
            stage if stage < READS_BEFORE_REFUSAL => read(stage),
            READS_BEFORE_REFUSAL => tagged_tool(
                "call-refused-write",
                "write",
                json!({"path":"outside.txt","content":"not approved\n"}),
            ),
            stage if stage == READS_BEFORE_REFUSAL + 1 => read(stage),
            _ => answer("the refused write bought no progress"),
        },
        83 => match stage {
            stage if stage < WRITES => tagged_tool(
                &format!("call-root-write-{stage}"),
                "write",
                json!({"path":format!("file-{stage:02}.txt"),"content":format!("root wrote {stage}\n")}),
            ),
            _ => answer("the root wrote forty-five files and finished"),
        },
        84 => match stage {
            stage if stage < SUCCESSFUL_READS => read(stage),
            _ => answer("the read-only root answered"),
        },
        // The retired `coding_prepare_candidate` has no replacement: a root that
        // only reads is the long run of successful calls this scenario needs.
        86 => match stage {
            stage if stage < REFUSED_CALLS => read(stage),
            _ => answer("the read-only root answered"),
        },
        85 => match stage {
            stage if stage < TOLERATED_READS => read(stage),
            _ => answer("the read-only root answered one call short of the stall"),
        },
        // The retired `coding_prepare_candidate` loop left no replacement, so
        // the refused call is the root's own write: `tracked.txt` exists at the
        // baseline, and a write that names it without the digest a fresh read
        // observed is refused before anything is written.
        66 => match stage {
            stage if stage < REFUSED_CALLS => refused_write(stage),
            _ => answer("the refused writes bought no progress"),
        },
        67 => match stage {
            stage if stage < READS_BEFORE_REFUSAL => read(stage),
            // The 95 reads admitted the digest the file actually has, so the
            // refusal here is the guarded replace's own precondition: the
            // digest the call carried is not the file's.
            stage if stage == READS_BEFORE_REFUSAL => tagged_tool(
                "call-refused-write",
                "write",
                json!({"path":"tracked.txt","content":"not approved\n","expected_digest":STALE_DIGEST}),
            ),
            stage if stage == READS_BEFORE_REFUSAL + 1 => read(stage),
            _ => answer("the refused write bought no progress"),
        },
        115 => match stage {
            stage if stage < REPEATED_READS => tagged_tool(
                &format!("read-{stage}"),
                "read",
                json!({"path":"tracked.txt"}),
            ),
            _ => answer("the repeated reads stopped the run"),
        },
        116 => match stage {
            0 => tagged_tool(
                "call-root-write",
                "write",
                json!({"path":"added.txt","content":"FIXED\n"}),
            ),
            stage if stage <= NO_PROGRESS_READS => tagged_tool(
                &format!("read-missing-{stage}"),
                "read",
                json!({"path":format!("missing-{stage}.txt")}),
            ),
            _ => answer("the run stopped making progress"),
        },
        117 => match stage {
            stage if stage < PRODUCTIVE_COMMANDS => tagged_tool(
                &format!("echo-{stage}"),
                "shell",
                json!({"command":format!("echo {stage}"),"description":format!("echo {stage}")}),
            ),
            _ => answer("the productive run finished"),
        },
        _ => answer("unexpected root-progress mode"),
    }
}

/// One root write the write authority refuses: a write that names an existing
/// file without the digest a fresh read observed writes nothing.
fn refused_write(stage: usize) -> String {
    tagged_tool(
        &format!("refused-write-{stage}"),
        "write",
        json!({"path":"tracked.txt","content":"not approved\n"}),
    )
}

/// One read-only root call: it changes nothing the job records.
fn read(stage: usize) -> String {
    tagged_tool(
        &format!("read-{stage}"),
        "read",
        json!({"path":"tracked.txt"}),
    )
}
