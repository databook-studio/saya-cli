use crate::{
    agent::runtime::{self, PromptOverrides},
    config::runtime::RuntimeConfig,
    render::{RenderFormat, TerminalEvent},
    stream_render::TerminalSink,
};
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_store::{AuditOperation, AuditStatus, SqliteStateStore};
use saya_types::{ConnectionError, QueryRequest};
use std::{path::PathBuf, time::Instant};

use super::{
    connection,
    output::{emit, failure, failure_message},
    state,
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn ask(
    prompt: Option<String>,
    file: Option<PathBuf>,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    can_prompt: bool,
    included_profiles: Vec<String>,
    state_db: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let prompt = super::query_input::input(prompt, file)?;
    if prompt.trim().is_empty() {
        return Err("ask requires a prompt or --file".into());
    }
    let cancellation = CancellationToken::new();
    let sink = TerminalSink::new(format);
    let work = runtime::run_prompt_with_sink(
        runtime,
        &prompt,
        approval,
        can_prompt,
        PromptOverrides {
            included_profiles,
            ..Default::default()
        },
        Vec::new(),
        &sink,
        cancellation.clone(),
        Some(state_db.clone()),
    );
    tokio::pin!(work);
    match tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => { cancellation.cancel(); return Ok(130); }
    } {
        Ok(_) => Ok(0),
        Err(error) => failure_message(5, error.to_string(), format),
    }
}

pub(super) async fn run(
    sql: Option<String>,
    file: Option<PathBuf>,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let sql = super::query_input::input(sql, file)?;
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
    let started = Instant::now();
    let profile_name = runtime.resolved.profile_name.as_deref().unwrap_or("none");
    let identity = state::identity(profile_name, profile, &runtime.cache_scope);
    let Some(connector) = connection::connector(profile, runtime, 4, format, can_prompt).await?
    else {
        state::audit(
            state_db,
            &identity,
            AuditOperation::Query,
            AuditStatus::Failure,
            started.elapsed(),
            None,
            None,
            format,
        )
        .await;
        return Ok(4);
    };
    match connector.connect().await {
        Err(error) => {
            state::audit(
                state_db,
                &identity,
                AuditOperation::Query,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
                format,
            )
            .await;
            failure(3, error, format)
        }
        Ok(()) => match connector
            .execute(QueryRequest::new(sql, runtime.resolved.max_rows))
            .await
        {
            Ok(result) => {
                state::audit(
                    state_db,
                    &identity,
                    AuditOperation::Query,
                    AuditStatus::Success,
                    started.elapsed(),
                    Some(result.rows.len()),
                    Some(result.truncated),
                    format,
                )
                .await;
                emit(TerminalEvent::QueryResult { result }, format);
                Ok(0)
            }
            Err(error) => {
                state::audit(
                    state_db,
                    &identity,
                    AuditOperation::Query,
                    AuditStatus::Failure,
                    started.elapsed(),
                    None,
                    None,
                    format,
                )
                .await;
                failure(4, error, format)
            }
        },
    }
}
