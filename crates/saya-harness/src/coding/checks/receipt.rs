use std::{fmt, path::Path};

use saya_agent::CancellationToken;
use saya_types::{CheckReceiptV1, CodingCheckOutcome, ManagedCheckoutId, Sha256Digest};

use crate::{
    coding::{
        commands::CommandOutputStatus,
        integration::{DEFAULT_DELTA_DEADLINE, candidate_source_digest},
        worktree::{OwnershipInput, verify_owned_checkout},
    },
    runner::ProgramOutcome,
};

use super::{CheckExecutionError, CheckInvocation};

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod outcome_tests;

/// Immutable evidence returned only after an owned check process was joined.
pub struct CheckReceipt {
    wire: CheckReceiptV1,
    checkout_id: ManagedCheckoutId,
    profile_id: String,
    program: String,
    process: CheckProcessOutcome,
    output: CheckOutputReference,
}

impl fmt::Debug for CheckReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CheckReceipt")
            .field("checkout_id", &self.checkout_id)
            .field("profile_id", &self.profile_id)
            .field("program", &self.program)
            .field("process", &self.process)
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

impl CheckReceipt {
    pub(super) fn from_run(
        invocation: &CheckInvocation,
        checkout_id: ManagedCheckoutId,
        wire: CheckReceiptV1,
        process: CheckProcessOutcome,
        output: CheckOutputReference,
        actual_program: String,
    ) -> Self {
        Self {
            wire,
            checkout_id,
            profile_id: invocation.profile_id().to_owned(),
            program: actual_program,
            process,
            output,
        }
    }

    pub fn wire_receipt(&self) -> &CheckReceiptV1 {
        &self.wire
    }

    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    /// The actual admitted bare program name reported by the joined runner.
    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn process(&self) -> &CheckProcessOutcome {
        &self.process
    }

    pub fn output(&self) -> &CheckOutputReference {
        &self.output
    }

    pub fn outcome(&self) -> CodingCheckOutcome {
        self.wire.outcome()
    }

    /// A check is successful only after a clean zero exit with no timeout,
    /// cancellation, signal, or surviving process group.
    pub fn is_successful(&self) -> bool {
        self.outcome() == CodingCheckOutcome::KnownExit { code: 0 }
            && self.process.signal.is_none()
            && !self.process.timed_out
            && !self.process.cancelled
            && !self.process.killed_orphans
    }

    /// Verifies both checkout identity and source bytes before evidence reuse.
    pub async fn verify_at_use(
        &self,
        ownership: &OwnershipInput<'_>,
        cancellation: &CancellationToken,
    ) -> Result<(), CheckExecutionError> {
        if ownership.checkout_id != &self.checkout_id || ownership.job_id != self.wire.job_id() {
            return Err(CheckExecutionError::StaleReceipt);
        }
        let candidate = verify_owned_checkout(ownership, cancellation).await?;
        let source = candidate_source_digest(
            Path::new(candidate.path().as_str()),
            cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CheckExecutionError::Candidate)?;
        if &source != self.wire.source_sha256() {
            return Err(CheckExecutionError::StaleReceipt);
        }
        Ok(())
    }
}

/// The process facts copied from the joined `RunProgram` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckProcessOutcome {
    exit_code: Option<i32>,
    signal: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    killed_orphans: bool,
}

impl CheckProcessOutcome {
    pub(super) fn from_program(outcome: &ProgramOutcome) -> Self {
        Self {
            exit_code: outcome.exit_code,
            signal: outcome.signal,
            timed_out: outcome.killed_by_timeout,
            cancelled: outcome.cancelled,
            killed_orphans: outcome.killed_orphans,
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    pub fn signal(&self) -> Option<i32> {
        self.signal
    }

    pub fn timed_out(&self) -> bool {
        self.timed_out
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn killed_orphans(&self) -> bool {
        self.killed_orphans
    }
}

/// Bounded references to the command owner's already-redacted stream files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutputReference {
    directory_id: String,
    stored_bytes: u64,
    raw_total_bytes: u64,
    dropped_bytes: u64,
    capped: bool,
    complete: bool,
}

impl CheckOutputReference {
    pub(super) fn from_status(directory_id: String, status: &CommandOutputStatus) -> Self {
        Self {
            directory_id,
            stored_bytes: status.stored_bytes,
            raw_total_bytes: status.raw_total_bytes,
            dropped_bytes: status.dropped_bytes,
            capped: status.capped,
            complete: status.error.is_none() && !status.capped && status.dropped_bytes == 0,
        }
    }

    /// Relative opaque leaf under the caller's private check output root.
    pub fn directory_id(&self) -> &str {
        &self.directory_id
    }

    /// Names the two fixed stream files under `directory_id`.
    pub fn stream_names(&self) -> [&'static str; 2] {
        ["stdout", "stderr"]
    }

    pub fn stored_bytes(&self) -> u64 {
        self.stored_bytes
    }

    pub fn raw_total_bytes(&self) -> u64 {
        self.raw_total_bytes
    }

    pub fn dropped_bytes(&self) -> u64 {
        self.dropped_bytes
    }

    pub fn capped(&self) -> bool {
        self.capped
    }

    pub fn complete(&self) -> bool {
        self.complete
    }
}

pub(super) fn output_reference(
    directory_id: String,
    status: &CommandOutputStatus,
) -> CheckOutputReference {
    CheckOutputReference::from_status(directory_id, status)
}

pub(super) fn coding_outcome(outcome: &ProgramOutcome) -> CodingCheckOutcome {
    if outcome.killed_by_timeout
        || outcome.cancelled
        || outcome.signal.is_some()
        || outcome.killed_orphans
    {
        return CodingCheckOutcome::Uncertain;
    }
    match outcome.exit_code {
        Some(code) => CodingCheckOutcome::KnownExit { code },
        None => CodingCheckOutcome::Unknown,
    }
}

pub(super) fn make_wire_receipt(
    invocation: &CheckInvocation,
    job_id: saya_types::JobId,
    source_sha256: Sha256Digest,
    argv_sha256: Sha256Digest,
    outcome: CodingCheckOutcome,
) -> Result<CheckReceiptV1, saya_types::ReceiptError> {
    CheckReceiptV1::new(
        job_id,
        source_sha256,
        invocation.command_id().to_owned(),
        argv_sha256,
        invocation.toolchain().unwrap_or("unknown").to_owned(),
        invocation.environment_profile().to_owned(),
        outcome,
    )
}
