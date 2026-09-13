mod agent;
mod app;
mod chart;
mod cli;
mod commands;
mod config;
mod connection;
#[allow(dead_code)] // contract ops surface, not yet wired into an adapter
mod contracts;
mod grant_token;
#[cfg(test)]
mod grant_token_tests;
mod interactive;
mod render;
pub mod render_run;
mod render_usage;
mod runtime_profile;
mod slash;
mod stream_render;

#[cfg(test)]
mod privacy_tests;
mod profile_identity;
mod prompt_approval;
#[cfg(test)]
mod prompt_approval_tests;
mod state_path;

use clap::Parser;

pub use app::run;
pub use cli::{
    ClaimKindArg, Cli, Command, ConfigCommand, ConnectionCommand, ContractsCommand,
    ForgetReasonArg, FormatArg, GlobalOptions, ReviewDecisionArg, RunCommand, ThemeArg,
};
pub use commands::{capture_output_start, capture_output_take, run_contracts, run_management};
pub use config::runtime::{RuntimeConfig, approval_name, load_with_sources};
pub use interactive::session_paths::{default_session_dir, resolve_session_dir};
pub use interactive::{Session, SessionAction, SessionState};
pub use profile_identity::profile_identity;
pub use render::{
    ContractClaimView, ContractConflictView, ContractQueueItemView, ContractView, RenderFormat,
    TerminalEvent, render_event,
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
