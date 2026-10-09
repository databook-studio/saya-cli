//! The `saya run` CLI surface, driven through the real binary (the
//! `tests/mvp` pattern): a scratch `SAYA_CONFIG_HOME`, `SAYA_RUNS_DIR`, and
//! `SAYA_STATE_DB` per test, and a scripted local HTTP provider standing in
//! for the model (`tests/mvp/provider.rs` is the recipe).
//!
//! Seven guarantees:
//! 1. A headless `saya run` without `--allow` refuses before anything exists
//!    — no run directory, no prompt.
//! 2. A run paused by its wall-clock budget exits 6, and a later resume
//!    carries the same elapsed interval without invoking the provider again.
//! 3. Ctrl-C cancels a run and exits 130.
//! 4. `saya run list` and `saya run show <id>` render a run that exists; an
//!    unknown id fails cleanly, never a panic.
//! 5. `--allow` with an unknown scope name is a usage error (2).
//! 6. A plan asking for more than `--allow` granted is refused, naming the
//!    missing scope.
//! 7. On a real terminal the bound plan is approved once and the run
//!    proceeds — no further interaction; a refusal refuses with exit 2.
//! 8. `--allow none` states the empty scope set: a read-only run starts and
//!    completes with no write capability approved.

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command as ProcessCommand, Output, Stdio},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

include!("run_cli/support/server.rs");
include!("run_cli/support/environment.rs");
include!("run_cli/cases/plan_approval.rs");
include!("run_cli/cases/resume_wall_clock.rs");
include!("run_cli/cases/resume_state.rs");
include!("run_cli/cases/cancellation.rs");
include!("run_cli/cases/run_listing.rs");
include!("run_cli/support/json_lines.rs");
include!("run_cli/cases/ndjson.rs");
include!("run_cli/cases/nested_run.rs");
include!("run_cli/support/interactive.rs");
include!("run_cli/cases/interactive_approval.rs");
include!("run_cli/support/runner_config.rs");
include!("run_cli/cases/runner_staged.rs");
