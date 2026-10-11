use std::num::NonZeroU64;

use saya_types::MAX_RECEIPT_REFERENCE_BYTES;

use crate::runner::{MAX_ARG_BYTES, MAX_ARG_COUNT, validate_argv};

mod execute;
mod receipt;

pub use execute::{CheckExecutionInput, MAX_CHECK_TIMEOUT_SECONDS, execute_check};
pub use receipt::{CheckOutputReference, CheckProcessOutcome, CheckReceipt};

/// Why check execution or receipt reuse could not be completed honestly.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CheckExecutionError {
    #[error("the check invocation is invalid or exceeds its bound")]
    InvalidInvocation,
    #[error("the candidate and narrowed child authority do not match")]
    AuthorityMismatch,
    #[error("the selected program is outside the narrowed child authority")]
    ProgramDenied,
    #[error("the candidate could not be verified")]
    Ownership(#[from] crate::coding::worktree::OwnershipError),
    #[error("the published candidate snapshot could not be verified")]
    Snapshot(#[from] crate::coding::snapshot::SnapshotError),
    #[error("the candidate source could not be fingerprinted")]
    Candidate(#[from] crate::coding::integration::CandidateError),
    #[error("the command could not be confined to the candidate")]
    Confinement(#[from] crate::coding::worktree::ConfinementError),
    #[error("the host does not prove command confinement")]
    ConfinementUnsupported,
    #[error("the check output root is not a trusted directory outside the candidate")]
    OutputRoot,
    #[error("the command owner did not admit the check")]
    CommandStart(#[from] crate::coding::commands::CommandStartError),
    #[error("the supervised check did not settle within its bound")]
    CommandDidNotSettle,
    #[error("the command output reference or status is unavailable")]
    OutputUnavailable,
    #[error("the supervised check failed before an outcome was available")]
    Command(#[from] crate::coding::commands::CommandSessionError),
    #[error("the candidate changed while the check was running")]
    SourceChanged,
    #[error("the receipt no longer matches this candidate")]
    StaleReceipt,
    #[error("the check receipt could not be constructed")]
    Receipt(#[from] saya_types::ReceiptError),
}

/// Coordinator-selected check data. This value carries no command authority.
pub struct CheckInvocation {
    profile_id: String,
    command_id: String,
    program: String,
    argv: Vec<String>,
    timeout_seconds: NonZeroU64,
    toolchain: Option<String>,
    environment_profile: String,
}

impl CheckInvocation {
    pub fn new(
        profile_id: String,
        command_id: String,
        program: String,
        argv: Vec<String>,
        timeout_seconds: NonZeroU64,
        toolchain: Option<String>,
        environment_profile: String,
    ) -> Result<Self, CheckExecutionError> {
        if !valid_reference(&profile_id)
            || !valid_reference(&command_id)
            || !valid_reference(&program)
            || !valid_reference(&environment_profile)
            || toolchain
                .as_ref()
                .is_some_and(|value| !valid_reference(value))
            || timeout_seconds.get() > MAX_CHECK_TIMEOUT_SECONDS
            || argv.len() > MAX_ARG_COUNT
            || argv.iter().any(|arg| arg.len() > MAX_ARG_BYTES)
            || validate_argv(&argv).is_err()
        {
            return Err(CheckExecutionError::InvalidInvocation);
        }
        Ok(Self {
            profile_id,
            command_id,
            program,
            argv,
            timeout_seconds,
            toolchain,
            environment_profile,
        })
    }

    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn command_id(&self) -> &str {
        &self.command_id
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    pub fn timeout_seconds(&self) -> NonZeroU64 {
        self.timeout_seconds
    }

    pub fn toolchain(&self) -> Option<&str> {
        self.toolchain.as_deref()
    }

    pub fn environment_profile(&self) -> &str {
        &self.environment_profile
    }
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RECEIPT_REFERENCE_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}
