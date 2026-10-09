use super::super::EXIT_SAFETY;
use super::super::run_binding;
use super::super::run_outcome::{Replay, RunOutcome};
use super::super::{bindings, run_report};
use super::{ConnectedReplay, RunRequest, support};
use crate::commands::output::{emit, failure};
use crate::config::runtime::RuntimeConfig;
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{AuditStatus, InvestigationRepository, SqliteStateStore};
use saya_types::{ConnectionError, EvidenceSource, ExecutionEvidence, ExecutionEvidenceArgs};

pub(super) async fn execute(
    repo: &InvestigationRepository,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    state_db: &SqliteStateStore,
    request: RunRequest<'_>,
    connected: ConnectedReplay,
) -> Result<super::super::run_outcome::RunOutcome, Box<dyn std::error::Error>> {
    let ConnectedReplay {
        id,
        definition,
        bound,
        binding,
        review,
        connector,
        request_query,
        started,
        started_unix_ms,
    } = connected;
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
    if support::is_cancelled(request.cancellation) {
        support::audit(
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
        return support::no_replay(failure(EXIT_SAFETY, error, format));
    }
    match execution.result {
        Ok(result) => {
            support::audit(
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
            if support::is_cancelled(request.cancellation) {
                return support::stopped(format);
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
                    finished_unix_ms: support::unix_now_ms(),
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
                run_binding::refresh_binding(
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
            support::audit(
                state_db,
                &review.identity,
                AuditStatus::Failure,
                &started,
                None,
                None,
                format,
            )
            .await;
            support::no_replay(failure(EXIT_SAFETY, error, format))
        }
    }
}
