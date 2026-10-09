use super::super::run_outcome::RunOutcome;
use super::super::{EXIT_INVESTIGATION_ERROR, EXIT_SAFETY, fingerprint, params, run_binding};
use super::support;
use super::{ConnectedReplay, InitialReplay, RunRequest, Stage};
use crate::commands::{
    connection,
    output::{failure, failure_message},
};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_connectors::{prepare_for_dialect, sql_references};
use saya_store::{AuditStatus, SqliteStateStore};
use saya_types::QueryRequest;

pub(super) async fn connect(
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
    request: &RunRequest<'_>,
    initial: InitialReplay<'_>,
) -> Result<Stage<ConnectedReplay>, Box<dyn std::error::Error>> {
    let InitialReplay {
        profile,
        id,
        definition,
        bound,
        binding,
        mut review,
        started,
    } = initial;
    macro_rules! finish {
        ($result:expr) => {
            return match $result {
                Ok(outcome) => Ok(super::Stage::Finished(Box::new(outcome))),
                Err(error) => Err(error),
            }
        };
    }

    let Some(connector) =
        connection::connector(profile, runtime, EXIT_SAFETY, format, can_prompt).await?
    else {
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
        finish!(Ok(RunOutcome::plain(EXIT_SAFETY)));
    };
    if support::is_cancelled(request.cancellation) {
        finish!(support::stopped(format));
    }
    // Capability honesty (invariant 2): an engine without native binding
    // refuses with any parameters before it connects or executes — the
    // same refusal the safety layer would raise, one step earlier.
    if !bound.is_empty() && !connector.supports_parameters() {
        finish!(support::no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            format!(
                "parameters are not supported for {}; run a fixed SQL investigation instead",
                params::parameters_engine_name(definition.dialect)
            ),
            format,
        )));
    }
    let connection_result = connector.connect().await;
    if support::is_cancelled(request.cancellation) {
        finish!(support::stopped(format));
    }
    if let Err(error) = connection_result {
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
        finish!(support::no_replay(failure(3, error, format)));
    }
    // The run re-gates the SQL through the same read-only preparation the
    // execution uses: a document tampered into write SQL is refused by the
    // safety layer, before any review decision or query (invariant 1).
    if let Err(error) = prepare_for_dialect(&definition.sql, 1, definition.dialect) {
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
        finish!(support::no_replay(failure(EXIT_SAFETY, error, format)));
    }
    // Review authority is the SQL itself (A2): the referenced parts are
    // recomputed on every run and the stored `objects` field stays
    // informational, so imported metadata can never weaken the review.
    let references = sql_references(&definition.sql, definition.dialect);
    let analysis = if fingerprint::needs_schema(references.as_ref()) {
        if support::is_cancelled(request.cancellation) {
            finish!(support::stopped(format));
        }
        let schema_result = connector.schema().await;
        if support::is_cancelled(request.cancellation) {
            finish!(support::stopped(format));
        }
        match schema_result {
            Ok(tree) => fingerprint::analyze(Some(&tree), definition.dialect, references.as_ref()),
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
                finish!(support::no_replay(failure(3, error, format)));
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
            finish!(support::no_replay(failure_message(
                EXIT_INVESTIGATION_ERROR,
                format!(
                    "schema review unavailable: {reason}; pass --revalidate to run without a verified schema review"
                ),
                format,
            )));
        }
        fingerprint::Analysis::Unverifiable(_) => None,
    };
    // A stale review is refused before anything executes (invariant 3).
    let reasons = run_binding::staleness(
        binding.as_ref(),
        &definition,
        &review.target,
        &review.identity,
        review.fingerprint.as_deref(),
    );
    if !reasons.is_empty() && !request.revalidate {
        finish!(support::no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            run_binding::stale_message(&reasons),
            format,
        )));
    }

    let started_unix_ms = support::unix_now_ms();
    let request_query = if bound.is_empty() {
        QueryRequest::new(definition.sql.clone(), runtime.resolved.max_rows)
    } else {
        QueryRequest::with_params(
            definition.sql.clone(),
            runtime.resolved.max_rows,
            bound.clone(),
        )
    };
    if support::is_cancelled(request.cancellation) {
        finish!(support::stopped(format));
    }
    Ok(Stage::Continue(super::ConnectedReplay {
        id,
        definition,
        bound,
        binding,
        review,
        connector,
        request_query,
        started,
        started_unix_ms,
    }))
}
