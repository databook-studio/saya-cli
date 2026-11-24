//! The session universe's composition tests: what refuses at composition, the
//! containment facts the red tests pin, and the startup notices.
//!
//! The universe's advertised tool surface — `definitions`, the shell and its
//! sandboxes, the executor — went with the deleted turn body; these pins read
//! only what composition itself states.

use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedHostCommands, ResolvedJobs, ResolvedMemory, ThemeChoice,
};
use std::fs;
use std::path::PathBuf;

use super::SessionUniverse;

include!("tests/harness.rs");

include!("tests/workspace_unavailable.rs");

// ---------------------------------------------------------------------------
// the lock, and the session state boundary
// ---------------------------------------------------------------------------

include!("tests/acquisition_lifecycle.rs");

include!("tests/deletion_and_tombstone.rs");

include!("tests/resumed_session_scope.rs");

include!("tests/workspace_notices.rs");
