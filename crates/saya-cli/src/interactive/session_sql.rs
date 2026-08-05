use crate::{
    config::runtime::RuntimeConfig,
    render::{RenderFormat, TerminalEvent, render_event},
};
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::QueryRequest;

/// Executes a raw SQL query against the session's currently-active profile.
pub(crate) async fn run(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    can_prompt: bool,
    format: RenderFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(name) = profile_name else {
        emit(
            TerminalEvent::Error {
                message: "No active profile. Use /connect <profile> first.".into(),
            },
            format,
        );
        return Ok(());
    };

    let profile = match runtime.named_profile(name) {
        Ok(profile) => profile,
        Err(error) => {
            emit(
                TerminalEvent::Error {
                    message: error.to_string(),
                },
                format,
            );
            return Ok(());
        }
    };

    let settings = ConnectorOptions {
        query_timeout_seconds: runtime.resolved.query_timeout_seconds,
        ..Default::default()
    };

    let connector = match build_connector_with_prompt(
        profile,
        &runtime.secret_resolver(),
        settings,
        can_prompt,
    )
    .await
    {
        Ok(connector) => connector,
        Err(error) => {
            emit(
                TerminalEvent::Error {
                    message: error.to_string(),
                },
                format,
            );
            return Ok(());
        }
    };

    if let Err(error) = connector.connect().await {
        emit(
            TerminalEvent::Error {
                message: error.to_string(),
            },
            format,
        );
        return Ok(());
    }

    match connector
        .execute(QueryRequest::new(
            sql.to_string(),
            runtime.resolved.max_rows,
        ))
        .await
    {
        Ok(result) => emit(TerminalEvent::QueryResult { result }, format),
        Err(error) => emit(
            TerminalEvent::Error {
                message: error.to_string(),
            },
            format,
        ),
    }

    Ok(())
}

fn emit(event: TerminalEvent, format: RenderFormat) {
    let rendered = render_event(&event, format);
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
}
