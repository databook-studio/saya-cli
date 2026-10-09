use super::super::session_emit;
use super::super::{
    session::commands::SessionAction,
    session::lifecycle::capacity::{SaveFailure, notice_after_save, save_with_explanation},
    session::lifecycle::resume::{SessionDefaults, block_on, load_session},
    session::runtime::SessionRuntime,
    session_request::PromptResult,
    turn_summary::{ProseOutcome, ToolTally, TurnSummary, prose_status},
};
use crate::budget_wording::{budget_dimension, budget_partial_message};
use crate::interactive::session::lifecycle::paths::default_session_dir;
use crate::{
    Cli, GlobalOptions, RenderFormat, RuntimeConfig, SessionState, config,
    render::{TerminalEvent, TurnStatus},
    slash::{SlashCommand, parse_slash_command},
};
use saya_store::{FsSessionStore, SqliteStateStore};
use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

include!("launch_context.rs");
include!("approval.rs");
include!("headless_context.rs");
include!("headless_loop.rs");
include!("action_context.rs");
include!("action_outcome.rs");
include!("action_surface.rs");
include!("action_runs.rs");
include!("action_grants.rs");
include!("action_sessions.rs");
include!("action_commands.rs");
include!("action_lifecycle.rs");
include!("action_dispatch.rs");
include!("headless_entry.rs");
include!("launch_prepare.rs");
include!("launch_terminal.rs");
include!("launch_headless.rs");
include!("launch_entry.rs");

#[cfg(test)]
#[path = "headless_receipt_tests.rs"]
mod session_headless_receipt_tests;
#[cfg(test)]
#[path = "headless_task_sync_tests.rs"]
mod session_headless_task_sync_tests;

include!("tests/report_outcome.rs");
include!("tests/turn_file.rs");
include!("tests/setup_turn.rs");
include!("tests/capacity_persist.rs");
