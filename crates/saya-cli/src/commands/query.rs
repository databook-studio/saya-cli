use crate::{
    agent_runtime::{self, PromptOverrides},
    config_runtime::RuntimeConfig,
    render::{RenderFormat, TerminalEvent},
};
use saya_agent::{AgentEvent, ApprovalPolicy};
use saya_types::{ConnectionError, QueryRequest};
use std::{fs, path::PathBuf};

use super::{
    connection,
    output::{emit, failure, failure_message},
};

pub(super) async fn ask(
    prompt: Option<String>,
    file: Option<PathBuf>,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    can_prompt: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let prompt = input(prompt, file)?;
    if prompt.trim().is_empty() {
        return Err("ask requires a prompt or --file".into());
    }
    match agent_runtime::run_prompt(
        runtime,
        &prompt,
        approval,
        can_prompt,
        PromptOverrides::default(),
    )
    .await
    {
        Ok(output) => {
            for event in output.events {
                if let Some(event) = agent_event(event) {
                    emit(event, format);
                }
            }
            Ok(0)
        }
        Err(error) => failure_message(5, error.to_string(), format),
    }
}

fn agent_event(event: AgentEvent) -> Option<TerminalEvent> {
    match event {
        AgentEvent::AssistantText { text } => Some(TerminalEvent::AssistantText { text }),
        AgentEvent::ToolRequested { name } => Some(TerminalEvent::ToolRequested { name }),
        AgentEvent::ToolCompleted { name, summary } => {
            Some(TerminalEvent::ToolCompleted { name, summary })
        }
        AgentEvent::ToolDenied { name, reason } => Some(TerminalEvent::ToolDenied { name, reason }),
        AgentEvent::Complete => None,
    }
}

pub(super) async fn run(
    sql: Option<String>,
    file: Option<PathBuf>,
    runtime: &RuntimeConfig,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let sql = input(sql, file)?;
    if sql.trim().is_empty() {
        return Err("query requires --sql or --file".into());
    }
    let Some(profile) = runtime.resolved.profile.as_ref() else {
        return failure(
            4,
            ConnectionError::InvalidConfiguration("query requires a selected profile".into()),
            format,
        );
    };
    let Some(connector) = connection::connector(profile, runtime, 4, format).await? else {
        return Ok(4);
    };
    match connector.connect().await {
        Err(error) => failure(3, error, format),
        Ok(()) => match connector
            .execute(QueryRequest::new(sql, runtime.resolved.max_rows))
            .await
        {
            Ok(result) => {
                emit(TerminalEvent::QueryResult { result }, format);
                Ok(0)
            }
            Err(error) => failure(4, error, format),
        },
    }
}

fn input(
    value: Option<String>,
    file: Option<PathBuf>,
) -> Result<String, Box<dyn std::error::Error>> {
    match (value, file) {
        (Some(value), None) => Ok(value),
        (None, Some(path)) => Ok(fs::read_to_string(path)?),
        (Some(_), Some(_)) => Err("provide a prompt or --file, not both".into()),
        (None, None) => Err("a prompt or --file is required".into()),
    }
}
