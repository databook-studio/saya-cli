//! H2 — the prompt, the grant, the per-call journal.
//!
//! Each test below names a seam H2 adds: the host prompt body, the name-only
//! grant, and the allow-side journal event. Written before any of them
//! exists; the failing output is pasted verbatim into `REPORT.md` before any
//! green edit.

#[path = "host_red_tests/grammar.rs"]
mod grammar;
#[path = "host_red_tests/grants.rs"]
mod grants;
#[path = "host_red_tests/journal.rs"]
mod journal;
#[path = "host_red_tests/prompt.rs"]
mod prompt;

use crate::approval_facts::{ApprovalFacts, HostFacts};
use std::path::PathBuf;

/// The fixed facts the host prompt states.
fn host_facts() -> ApprovalFacts {
    ApprovalFacts {
        host: Some(HostFacts {
            workspace_root: PathBuf::from("/home/user/proj"),
            timeout_seconds: 600,
            pass_env: Vec::new(),
        }),
        ..ApprovalFacts::default()
    }
}
