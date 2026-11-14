//! Process receipts: the recorded fact of one git subprocess, its outcome/effect
//! vocabulary, and the error-to-outcome mapping.

use super::super::command::{GitError, GitOperation};

pub(super) const RECEIPT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitProcessOutcome {
    Exited,
    TimedOut,
    Cancelled,
    SpawnFailed,
    WaitFailed,
    CaptureFailed,
    OutputLimited,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitProcessEffect {
    None,
    ProcessSucceeded,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct GitProcessReceipt {
    pub(in crate::coding::git) version: u32,
    pub(in crate::coding::git) service_identity: String,
    pub(in crate::coding::git) sequence: u64,
    pub(in crate::coding::git) request_kind: &'static str,
    pub(in crate::coding::git) operation: GitOperation,
    pub(in crate::coding::git) pid: Option<u32>,
    pub(in crate::coding::git) exit_code: Option<i32>,
    pub(in crate::coding::git) signal: Option<i32>,
    pub(in crate::coding::git) outcome: GitProcessOutcome,
    pub(in crate::coding::git) effect: GitProcessEffect,
    pub(in crate::coding::git) killed_orphans: Option<bool>,
    pub(in crate::coding::git) duration_ms: u64,
    pub(in crate::coding::git) stdout_bytes: u64,
    pub(in crate::coding::git) stderr_bytes: u64,
    pub(in crate::coding::git) stdout_truncated: bool,
    pub(in crate::coding::git) stderr_truncated: bool,
}

impl GitProcessReceipt {
    pub fn version(&self) -> u32 {
        self.version
    }
    pub fn service_identity(&self) -> &str {
        &self.service_identity
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn request_kind(&self) -> &'static str {
        self.request_kind
    }
    pub fn operation(&self) -> GitOperation {
        self.operation
    }
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
    pub fn signal(&self) -> Option<i32> {
        self.signal
    }
    pub fn outcome(&self) -> GitProcessOutcome {
        self.outcome
    }
    pub fn effect(&self) -> GitProcessEffect {
        self.effect
    }
    pub fn killed_orphans(&self) -> Option<bool> {
        self.killed_orphans
    }
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
    pub fn stdout_bytes(&self) -> u64 {
        self.stdout_bytes
    }
    pub fn stderr_bytes(&self) -> u64 {
        self.stderr_bytes
    }
    pub fn stdout_truncated(&self) -> bool {
        self.stdout_truncated
    }
    pub fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }
}

pub(super) fn outcome(error: &GitError) -> GitProcessOutcome {
    match error {
        GitError::TimedOut => GitProcessOutcome::TimedOut,
        GitError::Cancelled => GitProcessOutcome::Cancelled,
        GitError::Spawn(_) => GitProcessOutcome::SpawnFailed,
        GitError::Wait(_) => GitProcessOutcome::WaitFailed,
        GitError::CaptureUnavailable | GitError::InputUnavailable(_) => {
            GitProcessOutcome::CaptureFailed
        }
        GitError::OutputLimitExceeded => GitProcessOutcome::OutputLimited,
        _ => GitProcessOutcome::Unknown,
    }
}
pub(super) fn effect(
    operation: GitOperation,
    outcome: GitProcessOutcome,
    exit_code: Option<i32>,
) -> GitProcessEffect {
    if matches!(operation, GitOperation::Read) {
        GitProcessEffect::None
    } else if outcome == GitProcessOutcome::Exited && exit_code == Some(0) {
        GitProcessEffect::ProcessSucceeded
    } else {
        GitProcessEffect::Unknown
    }
}
