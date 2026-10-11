use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use saya_agent::CancellationToken;
use saya_types::{InterpreterScope, RunnerScope};
use sha2::{Digest, Sha256};

use crate::{
    coding::{
        commands::{CommandOutputStatus, CommandOwner, CommandSession},
        integration::{DEFAULT_DELTA_DEADLINE, candidate_source_digest},
        policy::ChildAuthorityView,
        worktree::{
            ConfinementError, OwnershipInput, VerifiedCheckout, confine_checkout,
            verify_owned_checkout,
        },
    },
    runner::SharedCredentialSource,
};

use super::receipt::{
    CheckProcessOutcome, CheckReceipt, coding_outcome, make_wire_receipt, output_reference,
};
use super::{CheckExecutionError, CheckInvocation, valid_reference};

pub const MAX_CHECK_TIMEOUT_SECONDS: u64 = 900;

/// Inputs from the trusted coordinator and the existing ownership/runner owners.
pub struct CheckExecutionInput<'a> {
    pub ownership: &'a OwnershipInput<'a>,
    pub authority: &'a ChildAuthorityView,
    pub invocation: &'a CheckInvocation,
    pub owner: &'a CommandOwner,
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
    let session = input.owner.start_with_output(
        runner,
        input.invocation.program().to_owned(),
        input.invocation.argv().to_vec(),
        input.invocation.timeout_seconds(),
        output_root.clone(),
    )?;
    let Some(output_dir) = session.output_dir() else {
        let _ = session.stop_and_join().await;
        return Err(CheckExecutionError::OutputUnavailable);
    };
    let output_status = wait_for_output_status(&session, timeout).await;
    let joined = match output_status {
        SettledOutput::Ready(status) => (Some(status), session.join().await),
        SettledOutput::Missing => (None, session.join().await),
        SettledOutput::Timeout => {
            let _ = session.stop_and_join().await;
            verify_candidate_still_owned(&input, &verified).await?;
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
    verify_candidate_still_owned(&input, &verified).await?;
    let after = candidate_source_digest(
        Path::new(verified.path().as_str()),
        input.cancellation,
        DEFAULT_DELTA_DEADLINE,
    )?;
    if after != before {
        return Err(CheckExecutionError::SourceChanged);
    }
    let outcome = joined.1?;
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
    Ok(CheckReceipt::from_run(
        input.invocation,
        verified.checkout_id().clone(),
        wire,
        CheckProcessOutcome::from_program(&outcome),
        output,
        outcome.program,
    ))
}

async fn verify_candidate_still_owned(
    input: &CheckExecutionInput<'_>,
    original: &VerifiedCheckout,
) -> Result<(), CheckExecutionError> {
    let current = verify_owned_checkout(input.ownership, input.cancellation).await?;
    if current.checkout_id() != original.checkout_id()
        || current.path() != original.path()
        || current.snapshot_id() != original.snapshot_id()
        || current.snapshot_sha256() != original.snapshot_sha256()
        || current.plan_sha256() != original.plan_sha256()
        || current.base_commit() != original.base_commit()
        || current.agent_id() != original.agent_id()
    {
        return Err(CheckExecutionError::SourceChanged);
    }
    Ok(())
}

enum SettledOutput {
    Ready(CommandOutputStatus),
    Missing,
    Timeout,
}

async fn wait_for_output_status(session: &CommandSession, timeout: Duration) -> SettledOutput {
    let wait = timeout.saturating_add(Duration::from_secs(2));
    match tokio::time::timeout(wait, async {
        loop {
            if let Some(status) = session.output_status() {
                return Some(status);
            }
            if session.is_finished() {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    {
        Ok(Some(status)) => SettledOutput::Ready(status),
        Ok(None) => SettledOutput::Missing,
        Err(_) => SettledOutput::Timeout,
    }
}

fn external_output_root(path: &Path, candidate: &Path) -> Result<PathBuf, CheckExecutionError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| CheckExecutionError::OutputRoot)?;
    if !metadata.is_dir() {
        return Err(CheckExecutionError::OutputRoot);
    }
    let output = fs::canonicalize(path).map_err(|_| CheckExecutionError::OutputRoot)?;
    let candidate = fs::canonicalize(candidate).map_err(|_| CheckExecutionError::OutputRoot)?;
    if output.as_path() != path || output.starts_with(candidate) {
        return Err(CheckExecutionError::OutputRoot);
    }
    Ok(output)
}

fn output_directory_id(
    directory: &Path,
    output_root: &Path,
) -> Result<String, CheckExecutionError> {
    let canonical =
        fs::canonicalize(directory).map_err(|_| CheckExecutionError::OutputUnavailable)?;
    let parent = canonical
        .parent()
        .ok_or(CheckExecutionError::OutputUnavailable)?;
    if parent.file_name().and_then(|name| name.to_str()) != Some("command-output")
        || parent.parent().is_none_or(|root| root != output_root)
    {
        return Err(CheckExecutionError::OutputUnavailable);
    }
    let id = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(CheckExecutionError::OutputUnavailable)?;
    if !valid_reference(id) {
        return Err(CheckExecutionError::OutputUnavailable);
    }
    Ok(id.to_owned())
}

fn digest_argv(argv: &[String]) -> Result<saya_types::Sha256Digest, CheckExecutionError> {
    let bytes = serde_json::to_vec(argv).map_err(|_| CheckExecutionError::InvalidInvocation)?;
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        encoded.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    saya_types::Sha256Digest::parse(&encoded).map_err(|_| CheckExecutionError::InvalidInvocation)
}
