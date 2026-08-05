mod agent;
mod app;
mod cli;
mod commands;
mod config;
mod connection;
mod interactive;
mod render;
mod runtime_profile;
mod slash;
mod stream_render;

#[cfg(test)]
mod privacy_tests;
mod profile_identity;
mod prompt_approval;
mod state_path;

use clap::Parser;

pub use app::run;
pub use cli::{Cli, Command, ConfigCommand, ConnectionCommand, FormatArg, GlobalOptions};
pub use config::runtime::{RuntimeConfig, approval_name, load_with_sources};
pub use interactive::session_paths::{default_session_dir, resolve_session_dir};
pub use interactive::{Session, SessionAction, SessionState};
pub use render::{RenderFormat, TerminalEvent, render_event};
pub use slash::{SlashCommand, parse_slash_command};
pub use state_path::resolve_state_db_path;

pub fn run_from_env() -> i32 {
    run(Cli::parse_from(std::env::args_os()))
}
