mod config;
mod connection;
mod output;
mod query;

use crate::{cli::Command, config_runtime::RuntimeConfig, render::RenderFormat};

pub async fn run(
    command: Command,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        Command::Config { command } => config::run(command, runtime, format),
        Command::Connection { command } => connection::run(command, runtime, format).await,
        Command::Ask { prompt, file } => query::ask(prompt, file, format),
        Command::Query { sql, file } => query::run(sql, file, runtime, format).await,
    }
}
