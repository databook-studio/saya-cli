// Unix-only: the pty harness behind `tui_pty_smoke.rs` is macOS/Linux, and
// the same ConPTY unknown applies here.
#![cfg(not(windows))]

//! A `--resume` LAUNCH, on a pty, over the surface T19's switch opened.
//!
//! Two cases share one fixture (`support`): the host posture the resumed
//! universe composes, which the status line must carry, and the state the
//! resumed record itself has — a 0.5.0 record, which opens read-only and whose
//! composer says what the next message does.
//!
//! Both cases read the NEW surface's words: T19 moved the launch onto it, and
//! the line front's opener — the composition, and the resume decision it acts
//! on — is the one both surfaces state their run from.

#[path = "resume_posture_pty/support.rs"]
mod support;

#[path = "resume_posture_pty/posture.rs"]
mod posture;

#[path = "resume_posture_pty/read_only.rs"]
mod read_only;
