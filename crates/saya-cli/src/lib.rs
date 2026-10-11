mod agent;
mod answering_receipts;
mod app;
mod approval_facts;
mod approval_text;
mod budget_wording;
mod chart;
mod cli;
mod commands;
mod config;
mod connection;
#[allow(dead_code)] // contract ops surface, not yet wired into an adapter
mod contracts;
mod demo;
mod grant_token;
#[cfg(test)]
mod grant_token_tests;
#[cfg(test)]
mod host_h3_red_tests;
#[cfg(test)]
mod host_h4_tests;
#[cfg(test)]
mod host_red_tests;
mod interactive;
mod local_metrics;
mod local_metrics_format;
#[cfg(test)]
mod local_metrics_format_tests;
mod local_metrics_index;
#[cfg(test)]
mod local_metrics_tests;
mod mcp;
mod metrics_sink;
mod open;
mod render;
pub mod render_run;
mod render_usage;
mod runtime_profile;
mod setup;
mod slash;
mod stream_render;
mod terminal_input;

#[cfg(test)]
mod privacy_tests;
mod profile_identity;
mod prompt_approval;
mod prompt_approval_metrics;
#[cfg(test)]
mod prompt_approval_tests;
mod state_path;

use clap::Parser;

pub use app::run;
pub use cli::{
    ClaimKindArg, Cli, CodingCommand, Command, ConfigCommand, ConnectionCommand, ContractsCommand,
    ForgetReasonArg, FormatArg, GlobalOptions, InvestigationCommand, ReviewDecisionArg, RunCommand,
    ThemeArg,
};
pub use commands::coding::{
    CodingApplication, CodingApprovalRequest, CodingAuthorization, CodingError, CodingJobRequest,
    CodingReview, CodingTask, NativeCodingJob, dispatch_coding,
};
pub use commands::{
    Replay, RunOutcome, capture_output_start, capture_output_take, run_contracts,
    run_investigation, run_investigation_outcome, run_management,
};
pub use config::runtime::{RuntimeConfig, approval_name, load_with_sources};
// The `TerminalEvent::DoctorTask` payload types: a public enum may not
// expose crate-only types, so they ride the same public re-export the
// contract views do.
pub use config::doctor_task::{CheckStatus, TaskCheck};

#[cfg(test)]
mod approval_facts_tests;
pub use interactive::session::lifecycle::paths::{default_session_dir, resolve_session_dir};
pub use interactive::{Session, SessionAction, SessionState};
pub use profile_identity::profile_identity;
pub use render::{
    ContractClaimView, ContractConflictView, ContractQueueItemView, ContractView, RenderFormat,
    ResumeEffectCode, TerminalEvent, render_event,
};
// The run event renderer's public seam: the parity test renders `RunEvent`
// lines with it, and `run_management` is the one dispatcher the slash
// adapters and the headless `saya run` commands share.
pub use render_run::render_run_event;
pub use slash::{SlashCommand, parse_slash_command};
pub use state_path::resolve_state_db_path;

pub fn run_from_env() -> i32 {
    run(Cli::parse_from(std::env::args_os()))
}
