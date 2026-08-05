mod config;
pub(crate) mod connection;
pub(crate) mod connection_schema;
mod connection_schema_cache;
mod output;
mod query;
mod query_input;
mod state;

use crate::{cli::Command, config::runtime::RuntimeConfig, render::RenderFormat};
use saya_agent::ApprovalPolicy;
use saya_store::SqliteStateStore;

pub async fn run(
    command: Command,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    can_prompt: bool,
    included_profiles: Vec<String>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let state = SqliteStateStore::new(crate::state_path::state_db_path());
    match command {
        Command::Config { command } => config::run(command, runtime, format),
        Command::Connection { command } => {
            connection::run(command, runtime, format, can_prompt, &state).await
        }
        Command::Ask { prompt, file } => {
            query::ask(
                prompt,
                file,
                runtime,
                format,
                approval,
                can_prompt,
                included_profiles,
                &state,
            )
            .await
        }
        Command::Query { sql, file } => {
            query::run(sql, file, runtime, format, can_prompt, &state).await
        }
    }
}

pub(crate) fn run_config_init(format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    config::run_init(format)
}
