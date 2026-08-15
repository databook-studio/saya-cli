mod config;
pub(crate) mod connection;
pub(crate) mod connection_schema;
mod connection_schema_cache;
mod connection_schema_reconcile;
mod contracts;
mod output;
mod preferences;
mod query;
mod query_input;
mod state;

use crate::{cli::Command, config::runtime::RuntimeConfig, render::RenderFormat};
use saya_agent::ApprovalPolicy;
use saya_store::SqliteStateStore;

pub use contracts::run_contracts;
pub use output::{capture_output_start, capture_output_take};
pub use preferences::run_preferences;
// Re-exported `pub(crate)` so the agent contract tools (2b-3a) reuse the single
// all-zero "no schema observed" fingerprint rather than inventing a second one.
pub(crate) use contracts::unobserved_fingerprint;
// Re-exported `pub(crate)` so the agent contract tools reuse the single
// identity-dropping `RetrievedContract → ContractView` mapping.
pub(crate) use contracts::contract_view;
// Re-exported `pub(crate)` so the agent contract tools load the cached schema
// the same way the CLI read commands do — one schema-lookup convention, not a
// second one that could disagree on "no cache" vs "empty cache". The write
// path (`remember`/`import`) keeps its own `cached_schema` (it resolves a
// fingerprint, not a classification); the read/classify paths use
// `cached_schema_availability` so a store error or undiscovered profile is
// `LiveSchemaUnavailable`, not a collapsed empty tree that would read `Stale`
// (the P1 bug).
pub(crate) use contracts::cached_schema_availability;

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
        Command::Contracts { command } => {
            contracts::run_contracts(command, runtime, format, &state).await
        }
        Command::Preferences { command } => {
            preferences::run_preferences(command, runtime, format, &state).await
        }
    }
}

pub(crate) fn run_config_init(format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    config::run_init(format)
}
