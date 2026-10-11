//! Wire outcome classification is independent of process execution and wrapper UI policy.

use super::{CodingCheckOutcome, ProgramOutcome, coding_outcome};
use crate::runner::StreamCapture;

fn outcome() -> ProgramOutcome {
    let stream = StreamCapture {
        text: String::new(),
        truncated: false,
        dropped_bytes: 0,
        total_bytes: 0,
    };
    ProgramOutcome {
        program: "check-helper".into(),
        pid: 1,
        exit_code: Some(0),
        signal: None,
        killed_by_timeout: false,
        cancelled: false,
        killed_orphans: false,
        duration_ms: 1,
        stdout: stream.clone(),
        stderr: stream,
    }
}

#[test]
fn abnormal_cleanup_never_produces_a_green_wire_exit() {
    assert_eq!(
        coding_outcome(&outcome()),
        CodingCheckOutcome::KnownExit { code: 0 }
    );
    for flag in 0..4 {
        let mut actual = outcome();
        match flag {
            0 => actual.killed_by_timeout = true,
            1 => actual.cancelled = true,
            2 => actual.signal = Some(9),
            _ => actual.killed_orphans = true,
        }
        assert_eq!(coding_outcome(&actual), CodingCheckOutcome::Uncertain);
    }
    let mut failing = outcome();
    failing.exit_code = Some(112);
    assert_eq!(
        coding_outcome(&failing),
        CodingCheckOutcome::KnownExit { code: 112 }
    );
    failing.exit_code = None;
    assert_eq!(coding_outcome(&failing), CodingCheckOutcome::Unknown);
}
