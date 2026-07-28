mod config;
pub(crate) mod connection;
mod output;
mod query;

use crate::{cli::Command, config_runtime::RuntimeConfig, render::RenderFormat};
use saya_agent::ApprovalPolicy;

pub async fn run(
    command: Command,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    can_prompt: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        Command::Config { command } => config::run(command, runtime, format),
        Command::Connection { command } => {
            connection::run(command, runtime, format, can_prompt).await
        }
        Command::Ask { prompt, file } => {
            query::ask(prompt, file, runtime, format, approval, can_prompt).await
        }
        Command::Query { sql, file } => query::run(sql, file, runtime, format, can_prompt).await,
    }
}
