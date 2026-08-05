//! Blocking helpers that execute a command and RETURN a renderable event,
//! instead of printing to stdout (which would corrupt the alternate screen).
//! The TUI renders the returned event into the transcript.

use crate::config::runtime::RuntimeConfig;
use crate::render::TerminalEvent;
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::QueryRequest;

/// Runs a raw SQL query against `profile_name` and returns the result (or an
/// error) as a `TerminalEvent`. Read-only + row-limit enforcement happen inside
/// the connector's `execute`. Never prompts (the TUI owns the screen).
pub(crate) async fn run_sql(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
) -> TerminalEvent {
    let Some(name) = profile_name else {
        return TerminalEvent::Error {
            message: "No active profile. Use /connect <profile> first.".into(),
        };
    };
    let profile = match runtime.named_profile(name) {
        Ok(profile) => profile,
        Err(error) => {
            return TerminalEvent::Error {
                message: error.to_string(),
            };
        }
    };
    let settings = ConnectorOptions {
        query_timeout_seconds: runtime.resolved.query_timeout_seconds,
        ..Default::default()
    };
    let connector =
        match build_connector_with_prompt(profile, &runtime.secret_resolver(), settings, false)
            .await
        {
            Ok(connector) => connector,
            Err(error) => {
                return TerminalEvent::Error {
                    message: error.to_string(),
                };
            }
        };
    if let Err(error) = connector.connect().await {
        return TerminalEvent::Error {
            message: error.to_string(),
        };
    }
    match connector
        .execute(QueryRequest::new(
            sql.to_string(),
            runtime.resolved.max_rows,
        ))
        .await
    {
        Ok(result) => TerminalEvent::QueryResult { result },
        Err(error) => TerminalEvent::Error {
            message: error.to_string(),
        },
    }
}
