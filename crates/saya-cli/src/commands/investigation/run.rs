//! `saya investigation run <id>` (S7, D4): replays a saved investigation
//! through the one query path — connector build, `connect`, the safety gate
//! inside `execute`, the same `QueryResult` renderer as `saya query` — with
//! an explicit local target and a review binding that goes stale when the
//! definition revision, target identity, or referenced schema changes. No
//! AI provider is constructed anywhere on this path, and the binding is
//! written only after a successful execution.

use super::{
    EXIT_INVESTIGATION_ERROR, EXIT_SAFETY, fingerprint,
    run_binding::{Review, refresh_binding, stale_message, staleness},
    store_failure,
};
use crate::commands::{
    connection,
    output::{emit, failure, failure_message},
    state,
};
use crate::config::runtime::RuntimeConfig;
use crate::profile_identity::profile_identity;
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{AuditOperation, AuditStatus, InvestigationRepository, SqliteStateStore};
use saya_types::{
    ConnectionError, EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryRequest,
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// What the dispatcher parsed for one replay.
pub(super) struct RunRequest<'a> {
    pub id: &'a str,
    pub connection: Option<&'a str>,
    pub revalidate: bool,
}

pub(super) async fn run(
    repo: &InvestigationRepository,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
    request: RunRequest<'_>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let id = match super::parse_investigation_id(request.id) {
        Ok(id) => id,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let definition = match repo.get(&id) {
        Ok(definition) => definition,
        Err(error) => return store_failure(error, id.as_str(), format),
    };
    let binding = match repo.get_binding(&id) {
        Ok(binding) => binding,
        Err(error) => return store_failure(error, id.as_str(), format),
    };
    // Target = `--connection` else the binding's profile; never the
    // active or default profile (invariant 1).
    let Some(target) = request
        .connection
        .or(binding.as_ref().map(|b| b.profile.as_str()))
    else {
        return failure_message(
            EXIT_INVESTIGATION_ERROR,
            "no local connection mapped: pass --connection <profile>".to_string(),
            format,
        );
    };
    let profile = match runtime.named_profile(target) {
        Ok(profile) => profile,
        Err(error) => {
            return failure(
                3,
                ConnectionError::invalid_configuration(error.to_string()),
                format,
            );
        }
    };
    if profile.dialect() != definition.dialect {
        let message = format!(
            "dialect mismatch: the investigation is saved for {} but profile {target:?} is {}",
            definition.dialect.as_str(),
            profile.dialect().as_str(),
        );
        return failure_message(EXIT_INVESTIGATION_ERROR, message, format);
    }
    let mut review = Review {
        target: target.to_string(),
        identity: profile_identity(target, profile, &runtime.cache_scope)
            .as_str()
            .to_owned(),
        fingerprint: None,
    };

    let started = Instant::now();
    let Some(connector) =
        connection::connector(profile, runtime, EXIT_SAFETY, format, can_prompt).await?
    else {
        audit(
            state_db,
            &review.identity,
            AuditStatus::Failure,
            &started,
            None,
            None,
            format,
        )
        .await;
        return Ok(EXIT_SAFETY);
    };
    if let Err(error) = connector.connect().await {
        audit(
            state_db,
            &review.identity,
            AuditStatus::Failure,
            &started,
            None,
            None,
            format,
        )
        .await;
        return failure(3, error, format);
    }
    // The schema is fetched only for definitions that reference objects —
    // without references there is no fingerprint to bind.
    if !definition.objects.is_empty() {
        match connector.schema().await {
            Ok(tree) => {
                review.fingerprint = fingerprint::combined_fingerprint(&tree, &definition.objects);
            }
            Err(error) => {
                audit(
                    state_db,
                    &review.identity,
                    AuditStatus::Failure,
                    &started,
                    None,
                    None,
                    format,
                )
                .await;
                return failure(3, error, format);
            }
        }
    }
    // A stale review is refused before anything executes (invariant 3).
    let reasons = staleness(
        binding.as_ref(),
        &definition,
        &review.target,
        &review.identity,
        review.fingerprint.as_deref(),
    );
    if !reasons.is_empty() && !request.revalidate {
        return failure_message(EXIT_INVESTIGATION_ERROR, stale_message(&reasons), format);
    }

    let started_unix_ms = unix_now_ms();
    match connector
        .execute(QueryRequest::new(
            definition.sql.clone(),
            runtime.resolved.max_rows,
        ))
        .await
    {
        Ok(result) => {
            audit(
                state_db,
                &review.identity,
                AuditStatus::Success,
                &started,
                Some(result.rows.len()),
                Some(result.truncated),
                format,
            )
            .await;
            let evidence = ExecutionEvidence::for_result(
                &result,
                ExecutionEvidenceArgs {
                    execution_id: ExecutionEvidence::new_execution_id(started_unix_ms, 0),
                    connection_label: review.target.clone(),
                    connection_identity: None,
                    dialect: definition.dialect,
                    max_rows: runtime.resolved.max_rows,
                    started_unix_ms,
                    finished_unix_ms: unix_now_ms(),
                    source: EvidenceSource::SavedInvestigation {
                        id: id.as_str().to_string(),
                        revision: definition.revision,
                    },
                },
            );
            emit(TerminalEvent::QueryResult { result }, format);
            let line = evidence.human_line();
            emit(TerminalEvent::Result { message: line }, format);
            refresh_binding(
                repo,
                format,
                &definition,
                binding.as_ref(),
                request.revalidate,
                &review,
            );
            Ok(0)
        }
        Err(error) => {
            audit(
                state_db,
                &review.identity,
                AuditStatus::Failure,
                &started,
                None,
                None,
                format,
            )
            .await;
            failure(EXIT_SAFETY, error, format)
        }
    }
}

/// Every audit on the replay path is a `Query` operation; only status, rows,
/// and truncation differ.
async fn audit(
    state_db: &SqliteStateStore,
    identity: &str,
    status: AuditStatus,
    started: &Instant,
    rows: Option<usize>,
    truncated: Option<bool>,
    format: RenderFormat,
) {
    state::audit(
        state_db,
        identity,
        AuditOperation::Query,
        status,
        started.elapsed(),
        rows,
        truncated,
        format,
    )
    .await;
}

/// Wall clock in unix milliseconds (0 before the epoch).
fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}
