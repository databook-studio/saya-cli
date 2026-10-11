use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use saya_agent::CancellationToken;
use saya_types::{InterpreterScope, RunnerScope};

use crate::{
    coding::{
        budget::{JobBudgetLedger, start_budgeted_with_output},
        commands::{CommandOutputCapture, CommandOutputError, CommandOwner, OutputStream},
        integration::{DEFAULT_DELTA_DEADLINE, candidate_source_digest},
        policy::ChildAuthorityView,
        worktree::{ConfinementError, OwnershipInput, confine_checkout, verify_owned_checkout},
    },
    runner::SharedCredentialSource,
};

use super::evidence::diagnostic_stream;
use super::execute_support::{
    SettledOutput, digest_argv, external_output_root, output_directory_id,
    verify_candidate_still_owned, wait_for_output_status,
};
use super::receipt::{
    CheckProcessOutcome, CheckReceipt, coding_outcome, make_wire_receipt, output_reference,
};
use super::{CheckDiagnostics, CheckExecutionError, CheckInvocation, identity::CheckLineage};

pub const MAX_CHECK_TIMEOUT_SECONDS: u64 = 900;

/// Inputs from the trusted coordinator and the existing ownership/runner owners.
pub struct CheckExecutionInput<'a> {
    pub ownership: &'a OwnershipInput<'a>,
    pub authority: &'a ChildAuthorityView,
    pub invocation: &'a CheckInvocation,
    pub owner: &'a CommandOwner,
    pub ledger: &'a JobBudgetLedger,
    pub cancellation: &'a CancellationToken,
    /// Trusted host-owned executable directory; profile data never selects it.
    pub program_dir: &'a Path,
    /// Existing private output root, canonical and outside the candidate.
    pub output_root: &'a Path,
    pub resolver: SharedCredentialSource,
}

/// Renews ownership, narrows the runner to this admitted program, joins it,
/// and returns evidence only when the candidate source stayed unchanged.
pub async fn execute_check(
    input: CheckExecutionInput<'_>,
) -> Result<CheckReceipt, CheckExecutionError> {
    Ok(execute_owned(input, None).await?.receipt)
}

/// Parses only immutable writer bytes associated with this joined real check.
pub async fn execute_check_with_diagnostics(
    input: CheckExecutionInput<'_>,
) -> Result<CheckDiagnostics, CheckExecutionError> {
    let format = input
        .invocation
        .diagnostic_format()
        .ok_or(CheckExecutionError::DiagnosticFormatRequired)?;
    let stream = diagnostic_stream(format)?;
    let run = execute_owned(input, Some(stream)).await?;
    Ok(CheckDiagnostics::from_run(
        run.receipt,
        format,
        run.capture,
        &run.root,
    ))
}

struct ExecutedCheck {
    receipt: CheckReceipt,
    capture: Option<Result<CommandOutputCapture, CommandOutputError>>,
    root: PathBuf,
}

async fn execute_owned(
    input: CheckExecutionInput<'_>,
    stream: Option<OutputStream>,
) -> Result<ExecutedCheck, CheckExecutionError> {
    if &input.ledger.job_id().await != input.ownership.job_id {
        return Err(CheckExecutionError::AuthorityMismatch);
    }
    let verified = verify_owned_checkout(input.ownership, input.cancellation).await?;
    let snapshot = input
        .ownership
        .artifacts
        .read_snapshot(verified.snapshot_id())?;
    let Some(authority_checkout) = input.authority.managed_checkout_id() else {
        return Err(CheckExecutionError::AuthorityMismatch);
    };
    if input.authority.job_id() != input.ownership.job_id
        || authority_checkout != verified.checkout_id()
        || verified.checkout_id() != input.ownership.checkout_id
        || input.authority.repository_id() != snapshot.repository_id()
        || input.authority.snapshot_id() != verified.snapshot_id()
        || snapshot.captured_manifest_sha256() != verified.snapshot_sha256()
        || snapshot.plan_manifest_sha256() != verified.plan_sha256()
    {
        return Err(CheckExecutionError::AuthorityMismatch);
    }
    let lineage = CheckLineage::from_run(&verified, snapshot.repository_id(), input.authority);
    let runner_scope = match input
        .authority
        .admit_runner_program(input.invocation.program())
    {
        Ok(()) => Some(
            RunnerScope::new(vec![input.invocation.program().to_owned()])
                .map_err(|_| CheckExecutionError::InvalidInvocation)?,
        ),
        Err(_) => None,
    };
    let interpreter_scope = if runner_scope.is_none() {
        if input
            .authority
            .admit_interpreter_program(input.invocation.program())
            .is_ok()
        {
            Some(
                InterpreterScope::new(vec![input.invocation.program().to_owned()])
                    .map_err(|_| CheckExecutionError::InvalidInvocation)?,
            )
        } else {
            None
        }
    } else {
        None
    };
    if runner_scope.is_none() && interpreter_scope.is_none() {
        return Err(CheckExecutionError::ProgramDenied);
    }

    let confined = match confine_checkout(&verified, input.program_dir) {
        Err(ConfinementError::Unsupported) => {
            return Err(CheckExecutionError::ConfinementUnsupported);
        }
        result => result?,
    };
    if confined.checkout_id() != verified.checkout_id() {
        return Err(CheckExecutionError::AuthorityMismatch);
    }
    let output_root = external_output_root(input.output_root, Path::new(verified.path().as_str()))?;
    let before = candidate_source_digest(
        Path::new(verified.path().as_str()),
        input.cancellation,
        DEFAULT_DELTA_DEADLINE,
    )?;
    let timeout = Duration::from_secs(input.invocation.timeout_seconds().get());
    let runner = confined
        .runner_tool(
            runner_scope,
            interpreter_scope,
            timeout,
            std::sync::Arc::clone(&input.resolver),
            output_root.clone(),
        )
        .with_cancellation(input.cancellation.clone());
    let session = start_budgeted_with_output(
        input.ledger,
        input.owner,
        runner,
        input.invocation.program().to_owned(),
        input.invocation.argv().to_vec(),
        input.invocation.timeout_seconds(),
        output_root.clone(),
    )
    .await?;
    let Some(output_dir) = session.session().output_dir() else {
        let _ = session.stop_and_join().await;
        return Err(CheckExecutionError::OutputUnavailable);
    };
    let output_status = wait_for_output_status(session.session(), timeout).await;
    // Capture is data, including typed failure. Always join before producing evidence.
    let capture = stream.map(|stream| session.session().capture_output(stream));
    let joined = match output_status {
        SettledOutput::Ready(status) => (Some(status), session.join().await),
        SettledOutput::Missing => (None, session.join().await),
        SettledOutput::Timeout => {
            let _ = session.stop_and_join().await;
            verify_candidate_still_owned(&input, &lineage).await?;
            let after = candidate_source_digest(
                Path::new(verified.path().as_str()),
                input.cancellation,
                DEFAULT_DELTA_DEADLINE,
            )?;
            if after != before {
                return Err(CheckExecutionError::SourceChanged);
            }
            return Err(CheckExecutionError::CommandDidNotSettle);
        }
    };
    verify_candidate_still_owned(&input, &lineage).await?;
    let after = candidate_source_digest(
        Path::new(verified.path().as_str()),
        input.cancellation,
        DEFAULT_DELTA_DEADLINE,
    )?;
    if after != before {
        return Err(CheckExecutionError::SourceChanged);
    }
    joined
        .1
        .settlement
        .map_err(CheckExecutionError::CommandSettlement)?;
    let outcome = joined.1.command?;
    if outcome.program != input.invocation.program() {
        return Err(CheckExecutionError::InvalidInvocation);
    }
    let status = joined.0.ok_or(CheckExecutionError::OutputUnavailable)?;
    let directory_id = output_directory_id(&output_dir, &output_root)?;
    let argv_sha256 = digest_argv(input.invocation.argv())?;
    let check_outcome = coding_outcome(&outcome);
    let wire = make_wire_receipt(
        input.invocation,
        input.ownership.job_id.clone(),
        before,
        argv_sha256,
        check_outcome,
    )?;
    let output = output_reference(directory_id, &status);
    let receipt = CheckReceipt::from_run(
        input.invocation,
        verified.checkout_id().clone(),
        wire,
        CheckProcessOutcome::from_program(&outcome),
        output,
        outcome.program,
        lineage,
    );
    Ok(ExecutedCheck {
        receipt,
        capture,
        root: PathBuf::from(verified.path().as_str()),
    })
}
