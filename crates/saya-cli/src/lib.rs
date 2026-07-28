mod agent_provider;
mod agent_runtime;
mod agent_tools;
mod app;
mod cli;
mod commands;
mod config_doctor;
mod config_runtime;
mod config_sources;
mod interactive;
mod render;
mod runtime_profile;
mod session_paths;
mod slash;

use clap::Parser;

pub use app::run;
pub use cli::{Cli, Command, ConfigCommand, ConnectionCommand, FormatArg, GlobalOptions};
pub use config_runtime::{RuntimeConfig, approval_name, load_with_sources};
pub use interactive::{Session, SessionAction, SessionState};
pub use render::{RenderFormat, TerminalEvent, render_event};
pub use session_paths::{default_session_dir, resolve_session_dir};
pub use slash::{SlashCommand, parse_slash_command};

pub fn run_from_env() -> i32 {
    run(Cli::parse_from(std::env::args_os()))
}
