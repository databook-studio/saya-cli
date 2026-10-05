//! `saya investigation run <id>` (S7, D4): replays a saved investigation
//! through the one query path — connector build, `connect`, the safety gate
//! inside `execute`, the same `QueryResult` renderer as `saya query` — with
//! an explicit local target and a review binding that goes stale when the
//! definition revision, target identity, or referenced schema changes. No
//! AI provider is constructed anywhere on this path, and the binding is
//! written only after a successful execution. The operation returns a typed
//! [`RunOutcome`] (C3/D12) so adapters capture the replay instead of
//! re-parsing rendered output.

use super::{
    EXIT_INVESTIGATION_ERROR, EXIT_SAFETY, bindings, fingerprint, params,
    run_binding::{Review, refresh_binding, stale_message, staleness},
    run_outcome::{Replay, RunOutcome},
    run_report, store_failure,
};
use crate::commands::{
    connection,
    output::{emit, failure, failure_message},
    state,
};
use crate::config::runtime::RuntimeConfig;
use crate::profile_identity::profile_identity;
use crate::render::{RenderFormat, TerminalEvent};
use saya_agent::CancellationToken;
use saya_connectors::{prepare_for_dialect, sql_references};
use saya_store::{AuditOperation, AuditStatus, InvestigationRepository, SqliteStateStore};
use saya_types::{
    ConnectionError, EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs, QueryRequest,
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// What the dispatcher parsed for one replay. The report fields (S12b)
/// carry an optional `--report` destination with its `--rows` opt-in and
/// `--overwrite` decision; `params` carries the raw `--param name=value`
/// bindings (B1f). `machine_driven` marks the MCP replay adapter's runs:
/// such a run can never write or complete review authority.
pub(super) struct RunRequest<'a> {
    pub id: &'a str,
    pub connection: Option<&'a str>,
    pub revalidate: bool,
    pub report: Option<&'a std::path::Path>,
    pub rows: Option<usize>,
    pub overwrite: bool,
    pub params: Vec<String>,
    pub machine_driven: bool,
    pub cancellation: Option<&'a CancellationToken>,
}

pub(super) async fn run(
    repo: &InvestigationRepository,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
    request: RunRequest<'_>,
) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    if is_cancelled(request.cancellation) {
        return stopped(format);
    }
    // Flag-usage refusals come first: they are command-line errors, checked
    // before any store, connection, or query work (invariant 1).
    if let Some(message) = run_report::usage_error(&request) {
        return no_replay(failure_message(EXIT_INVESTIGATION_ERROR, message, format));
    }
    let id = match super::parse_investigation_id(request.id) {
        Ok(id) => id,
        Err((code, message)) => return no_replay(failure_message(code, message, format)),
    };
    let definition = match repo.get(&id) {
        Ok(definition) => definition,
        Err(error) => return no_replay(store_failure(error, id.as_str(), format)),
    };
    // The parameter bindings parse against the declared specs (which the
    // definition read above supplies) before any profile or connection
    // work (invariant 2) — no connection is ever made with a bad binding:
    // unknown names, a malformed value, and a missing required parameter
    // are usage refusals.
    let bound = match bindings::bind_values(&definition.parameters, &request.params) {
        Ok(bound) => bound,
        Err((code, message)) => return no_replay(failure_message(code, message, format)),
    };
    let binding = match repo.get_binding(&id) {
        Ok(binding) => binding,
        Err(error) => return no_replay(store_failure(error, id.as_str(), format)),
    };
    // A machine-driven replay (the MCP tool) can never establish review
    // authority (FIX-S6a): the binding this same run just read must carry a
    // schema fingerprint — a human step wrote it — or the replay refuses
    // before anything executes (no re-read to disagree; id only, no values).
    if request.machine_driven
        && !binding
            .as_ref()
            .is_some_and(|binding| binding.reviewed_schema_fingerprint.is_some())
    {
        return no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            format!(
                "this investigation has not been reviewed on this machine yet; review it once with `saya investigation run {} --connection <profile>` (a human step) before a tool can replay it",
                id.as_str(),
            ),
            format,
        ));
    }
    // Target = `--connection` else the binding's profile; never the
    // active or default profile (invariant 1).
    let Some(target) = request
        .connection
        .or(binding.as_ref().map(|b| b.profile.as_str()))
    else {
        return no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            "no local connection mapped: pass --connection <profile>".to_string(),
            format,
        ));
    };
    let profile = match runtime.named_profile(target) {
        Ok(profile) => profile,
        Err(error) => {
            return no_replay(failure(
                3,
                ConnectionError::invalid_configuration(error.to_string()),
                format,
            ));
        }
    };
    if profile.dialect() != definition.dialect {
        let message = format!(
            "dialect mismatch: the investigation is saved for {} but profile {target:?} is {}",
            definition.dialect.as_str(),
            profile.dialect().as_str(),
        );
        return no_replay(failure_message(EXIT_INVESTIGATION_ERROR, message, format));
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
        return Ok(RunOutcome::plain(EXIT_SAFETY));
    };
    if is_cancelled(request.cancellation) {
        return stopped(format);
    }
    // Capability honesty (invariant 2): an engine without native binding
    // refuses with any parameters before it connects or executes — the
    // same refusal the safety layer would raise, one step earlier.
    if !bound.is_empty() && !connector.supports_parameters() {
        return no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            format!(
                "parameters are not supported for {}; run a fixed SQL investigation instead",
                params::parameters_engine_name(definition.dialect)
            ),
            format,
        ));
    }
    let connection_result = connector.connect().await;
    if is_cancelled(request.cancellation) {
        return stopped(format);
    }
    if let Err(error) = connection_result {
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
        return no_replay(failure(3, error, format));
    }
    // The run re-gates the SQL through the same read-only preparation the
    // execution uses: a document tampered into write SQL is refused by the
    // safety layer, before any review decision or query (invariant 1).
    if let Err(error) = prepare_for_dialect(&definition.sql, 1, definition.dialect) {
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
        return no_replay(failure(EXIT_SAFETY, error, format));
    }
    // Review authority is the SQL itself (A2): the referenced parts are
    // recomputed on every run and the stored `objects` field stays
    // informational, so imported metadata can never weaken the review.
    let references = sql_references(&definition.sql, definition.dialect);
    let analysis = if fingerprint::needs_schema(references.as_ref()) {
        if is_cancelled(request.cancellation) {
            return stopped(format);
        }
        let schema_result = connector.schema().await;
        if is_cancelled(request.cancellation) {
            return stopped(format);
        }
        match schema_result {
            Ok(tree) => fingerprint::analyze(Some(&tree), definition.dialect, references.as_ref()),
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
                return no_replay(failure(3, error, format));
            }
        }
    } else {
        fingerprint::analyze(None, definition.dialect, references.as_ref())
    };
    // An unverifiable review is refused before anything executes (A2);
    // `--revalidate` runs it once and binds no fingerprint, so the next run
    // is unverifiable again — never silently verified.
    review.fingerprint = match analysis {
        fingerprint::Analysis::Complete(combined) => combined,
        fingerprint::Analysis::Unverifiable(reason) if !request.revalidate => {
            return no_replay(failure_message(
                EXIT_INVESTIGATION_ERROR,
                format!(
                    "schema review unavailable: {reason}; pass --revalidate to run without a verified schema review"
                ),
                format,
            ));
        }
        fingerprint::Analysis::Unverifiable(_) => None,
    };
    // A stale review is refused before anything executes (invariant 3).
    let reasons = staleness(
        binding.as_ref(),
        &definition,
        &review.target,
        &review.identity,
        review.fingerprint.as_deref(),
    );
    if !reasons.is_empty() && !request.revalidate {
        return no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            stale_message(&reasons),
            format,
        ));
    }

    let started_unix_ms = unix_now_ms();
    let request_query = if bound.is_empty() {
        QueryRequest::new(definition.sql.clone(), runtime.resolved.max_rows)
    } else {
        QueryRequest::with_params(
            definition.sql.clone(),
            runtime.resolved.max_rows,
            bound.clone(),
        )
    };
    if is_cancelled(request.cancellation) {
        return stopped(format);
    }
    let execution = match request.cancellation {
        Some(cancellation) => {
            crate::interactive::sql_operation::cancellation::execute_with_cancellation_outcome(
                connector.as_ref(),
                request_query,
                cancellation,
            )
            .await
        }
        None => crate::interactive::sql_operation::cancellation::SettledQuery {
            result: connector.execute(request_query).await,
            cancel_request: None,
        },
    };
    if is_cancelled(request.cancellation) {
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
        let error = match execution.cancel_request.and_then(Result::err) {
            Some(error) => error,
            None => match execution.result {
                Err(error) => error,
                Ok(_) => ConnectionError::cancelled(),
            },
        };
        return no_replay(failure(EXIT_SAFETY, error, format));
    }
    match execution.result {
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
            // The query and its success audit are already real. A stop can
            // arrive while the audit is awaited; preserve that audit, but do
            // not promote the result into replay evidence or review state.
            if is_cancelled(request.cancellation) {
                return stopped(format);
            }
            let (param_names, params_sha256) = bindings::evidence_fields(&bound);
            let evidence = ExecutionEvidence::for_result(
                &result,
                ExecutionEvidenceArgs {
                    execution_id: ExecutionEvidence::new_execution_id(started_unix_ms, 0),
                    connection_label: review.target.clone(),
                    connection_identity: Some(review.identity.clone()),
                    dialect: definition.dialect,
                    max_rows: runtime.resolved.max_rows,
                    started_unix_ms,
                    finished_unix_ms: unix_now_ms(),
                    source: EvidenceSource::SavedInvestigation {
                        id: id.as_str().to_string(),
                        revision: definition.revision,
                    },
                },
            )
            .with_param_bindings(param_names, params_sha256);
            emit(
                TerminalEvent::QueryResult {
                    result: result.clone(),
                },
                format,
            );
            let line = evidence.human_line();
            emit(TerminalEvent::Result { message: line }, format);
            // A machine-driven replay never rewrites review authority: the
            // binding it read authorized this run, so success leaves it as is.
            if !request.machine_driven {
                refresh_binding(
                    repo,
                    format,
                    &definition,
                    binding.as_ref(),
                    request.revalidate,
                    &review,
                );
            }
            // The report (S12b) is written last, from this execution's
            // result and evidence, after the output is out (invariants 2
            // and 3); without `--report` this is the plain success exit.
            // The execution itself succeeded either way, so the outcome
            // carries the replay for the caller to capture (D12).
            let code = run_report::write(&result, &evidence, &request, format)?;
            Ok(RunOutcome {
                code,
                replay: Some(Replay {
                    result,
                    evidence,
                    sql: definition.sql.clone(),
                    connection: review.target.clone(),
                }),
            })
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
            no_replay(failure(EXIT_SAFETY, error, format))
        }
    }
}

fn is_cancelled(cancellation: Option<&CancellationToken>) -> bool {
    cancellation.is_some_and(CancellationToken::is_cancelled)
}

fn stopped(format: RenderFormat) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    no_replay(failure_message(
        EXIT_INVESTIGATION_ERROR,
        "the call was cancelled".to_string(),
        format,
    ))
}

/// Wraps a code-only output result — a refusal or failure whose diagnostic
/// is already emitted — as the no-replay outcome.
fn no_replay(
    result: Result<i32, Box<dyn std::error::Error>>,
) -> Result<RunOutcome, Box<dyn std::error::Error>> {
    result.map(RunOutcome::plain)
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
