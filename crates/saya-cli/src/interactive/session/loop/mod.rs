use super::super::session_emit;
use super::super::{
    session::commands::SessionAction,
    session::lifecycle::capacity::{SaveFailure, notice_after_save, save_with_explanation},
    session::lifecycle::resume::{SessionDefaults, block_on, load_session},
    session::runtime::SessionRuntime,
    turn_summary::{ProseOutcome, ToolTally, TurnSummary, prose_status},
};
use super::line_chat::{LineChat, LineSession, Said};
use crate::chat::ChatOutcome;
use crate::interactive::session::lifecycle::paths::default_session_dir;
use crate::{
    Cli, GlobalOptions, RenderFormat, RuntimeConfig, SessionState, config,
    render::{TerminalEvent, TurnStatus},
    slash::parse_slash_command,
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
include!("action_status.rs");
include!("action_sessions.rs");
include!("action_commands.rs");
include!("action_coding.rs");
include!("action_lifecycle.rs");
include!("action_dispatch.rs");
include!("headless_entry.rs");
include!("action_message.rs");
include!("session_facts_sync.rs");
include!("launch_prepare.rs");
include!("launch_surface.rs");
include!("launch_terminal.rs");
include!("launch_headless.rs");
include!("launch_entry.rs");

#[cfg(test)]
#[path = "headless_checklist_delta_tests.rs"]
mod session_headless_checklist_delta_tests;
#[cfg(test)]
#[path = "headless_receipt_tests.rs"]
mod session_headless_receipt_tests;
#[cfg(test)]
#[path = "headless_task_sync_tests.rs"]
mod session_headless_task_sync_tests;
#[cfg(test)]
#[path = "user_command_turn_tests.rs"]
mod session_user_command_turn_tests;

include!("tests/report_outcome.rs");
include!("tests/turn_file.rs");
include!("tests/setup_turn.rs");
include!("tests/status_turn.rs");
include!("tests/capacity_persist.rs");
