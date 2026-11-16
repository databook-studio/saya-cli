//! `saya job`: the lifecycle lane's entry point.
//!
//! The lane runs the same twenty lifecycle verbs `saya job` runs, through
//! the same one `run` — the grammar is declared once, so the two spellings
//! cannot drift. What the lane adds is its rule: the eight agent-shaped
//! controls parse here too (they are the same enum), and are refused before
//! any state is touched, naming the spelling that does work, rather than
//! accepted as a no-op.

use crate::{cli::CodingCommand, config::runtime::RuntimeConfig, render::RenderFormat};
use saya_agent::ApprovalPolicy;
use saya_store::SqliteStateStore;
use std::path::PathBuf;

pub(crate) async fn run(
    command: CodingCommand,
    repository: PathBuf,
    program_dir: PathBuf,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    store: &SqliteStateStore,
    approval_policy: ApprovalPolicy,
) -> Result<i32, Box<dyn std::error::Error>> {
    if let Some(refusal) = crate::cli::job_lane_refusal(&command) {
        return crate::commands::output::failure_message(2, refusal, format);
    }
    super::cli::run(
        command,
        repository,
        program_dir,
        runtime,
        format,
        store,
        approval_policy,
    )
    .await
}
