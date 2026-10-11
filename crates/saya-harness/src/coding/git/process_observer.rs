use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use super::{
    command::{GitError, GitOperation},
    process::GitOutput,
    process_journal::{GitProcessJournal, JournalAppendError, JournalState},
};

const MAX_RECEIPTS: usize = 256;
const RECEIPT_VERSION: u32 = 1;
const MAX_RECEIPT_BYTES: usize = 512;
const MAX_JOURNAL_BYTES: usize = 16 * 1024;

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
    pub(super) version: u32,
    pub(super) service_identity: String,
    pub(super) sequence: u64,
    pub(super) request_kind: &'static str,
    pub(super) operation: GitOperation,
    pub(super) pid: Option<u32>,
    pub(super) exit_code: Option<i32>,
    pub(super) signal: Option<i32>,
    pub(super) outcome: GitProcessOutcome,
    pub(super) effect: GitProcessEffect,
    pub(super) killed_orphans: Option<bool>,
    pub(super) duration_ms: u64,
    pub(super) stdout_bytes: u64,
    pub(super) stderr_bytes: u64,
    pub(super) stdout_truncated: bool,
    pub(super) stderr_truncated: bool,
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

#[derive(Debug, Clone)]
pub struct GitProcessSnapshot {
    service_identity: String,
    receipts: Vec<GitProcessReceipt>,
    omitted: u64,
    omitted_bytes: u64,
    incomplete: bool,
}

impl GitProcessSnapshot {
    pub fn service_identity(&self) -> &str {
        &self.service_identity
    }
    pub fn receipts(&self) -> &[GitProcessReceipt] {
        &self.receipts
    }
    pub fn omitted(&self) -> u64 {
        self.omitted
    }
    pub fn omitted_bytes(&self) -> u64 {
        self.omitted_bytes
    }
    pub fn complete(&self) -> bool {
        self.omitted == 0 && !self.incomplete
    }
}

pub(super) struct GitProcessObserver {
    identity: String,
    journal: Arc<GitProcessJournal>,
    state: Mutex<State>,
}

struct State {
    next_sequence: u64,
    receipts: Vec<GitProcessReceipt>,
    omitted: u64,
    retained_bytes: usize,
    omitted_bytes: u64,
    incomplete: bool,
    session_active: bool,
}

impl GitProcessObserver {
    pub(super) fn new(
        identity: String,
        journal: Arc<GitProcessJournal>,
        journal_state: JournalState,
    ) -> Arc<Self> {
        let next_sequence = journal_state
            .receipts
            .iter()
            .filter(|receipt| receipt.service_identity == identity)
            .map(|receipt| receipt.sequence)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Arc::new(Self {
            identity,
            journal,
            state: Mutex::new(State {
                next_sequence,
                retained_bytes: journal_state.retained_bytes,
                receipts: journal_state.receipts,
                omitted: u64::from(journal_state.incomplete),
                omitted_bytes: 0,
                incomplete: journal_state.incomplete,
                session_active: journal_state.session_started,
            }),
        })
    }

    pub(super) fn begin(
        self: &Arc<Self>,
        request_kind: &'static str,
        operation: GitOperation,
    ) -> Observation {
        let mut state = self.lock();
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        Observation {
            observer: Arc::clone(self),
            receipt: GitProcessReceipt {
                version: RECEIPT_VERSION,
                service_identity: self.identity.clone(),
                sequence,
                request_kind,
                operation,
                pid: None,
                exit_code: None,
                signal: None,
                outcome: GitProcessOutcome::Unknown,
                effect: effect(operation, GitProcessOutcome::Unknown, None),
                killed_orphans: None,
                duration_ms: 0,
                stdout_bytes: 0,
                stderr_bytes: 0,
                stdout_truncated: false,
                stderr_truncated: false,
            },
            started: Instant::now(),
            finished: false,
        }
    }

    pub(super) fn snapshot(&self) -> GitProcessSnapshot {
        let state = self.lock();
        GitProcessSnapshot {
            service_identity: self.identity.clone(),
            receipts: state.receipts.clone(),
            omitted: state.omitted,
            omitted_bytes: state.omitted_bytes,
            incomplete: state.incomplete,
        }
    }

    fn push(&self, receipt: GitProcessReceipt) {
        let persisted = self.journal.append(&receipt);
        let mut state = self.lock();
        let bytes = receipt_bytes(&receipt);
        if let Err(error) = persisted {
            state.incomplete = true;
            if !matches!(error, JournalAppendError::AlreadyIncomplete) {
                state.omitted = state.omitted.saturating_add(1);
                state.omitted_bytes = state.omitted_bytes.saturating_add(bytes as u64);
            }
        }
        if state.receipts.len() == MAX_RECEIPTS
            || bytes > MAX_RECEIPT_BYTES
            || state.retained_bytes.saturating_add(bytes) > MAX_JOURNAL_BYTES
        {
            state.omitted = state.omitted.saturating_add(1);
            state.omitted_bytes = state.omitted_bytes.saturating_add(bytes as u64);
        } else {
            state.retained_bytes += bytes;
            state.receipts.push(receipt);
        }
    }

    pub(super) fn close(&self) {
        let should_close = {
            let state = self.lock();
            state.session_active && !state.incomplete
        };
        if !should_close {
            return;
        }
        let result = self.journal.close_session();
        let mut state = self.lock();
        if result.is_err() {
            state.incomplete = true;
        } else {
            state.session_active = false;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(super) struct Observation {
    observer: Arc<GitProcessObserver>,
    receipt: GitProcessReceipt,
    started: Instant,
    finished: bool,
}

impl Observation {
    pub(super) fn pid(&mut self, pid: u32) {
        self.receipt.pid = Some(pid);
    }
    pub(super) fn settled(&mut self, duration_ms: u64, killed_orphans: bool) {
        self.receipt.duration_ms = duration_ms;
        self.receipt.killed_orphans = Some(killed_orphans);
    }
    pub(super) fn capture(
        &mut self,
        stdout: u64,
        stderr: u64,
        stdout_truncated: bool,
        stderr_truncated: bool,
    ) {
        self.receipt.stdout_bytes = stdout;
        self.receipt.stderr_bytes = stderr;
        self.receipt.stdout_truncated = stdout_truncated;
        self.receipt.stderr_truncated = stderr_truncated;
    }
    pub(super) fn finish(&mut self, result: &Result<GitOutput, GitError>) {
        if self.receipt.duration_ms == 0 {
            self.receipt.duration_ms = self
                .started
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX);
        }
        match result {
            Ok(output) => {
                self.receipt.exit_code = output.exit_code;
                self.receipt.signal = output.signal;
                self.receipt.outcome = GitProcessOutcome::Exited;
            }
            Err(error) => self.receipt.outcome = outcome(error),
        }
        self.receipt.effect = effect(
            self.receipt.operation,
            self.receipt.outcome,
            self.receipt.exit_code,
        );
        self.finished = true;
        self.observer.push(self.receipt.clone());
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        if !self.finished {
            self.receipt.duration_ms = self
                .started
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX);
            self.receipt.outcome = GitProcessOutcome::Unknown;
            self.receipt.effect = effect(self.receipt.operation, self.receipt.outcome, None);
            self.observer.push(self.receipt.clone());
        }
    }
}

fn outcome(error: &GitError) -> GitProcessOutcome {
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
fn effect(
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

fn receipt_bytes(receipt: &GitProcessReceipt) -> usize {
    128usize
        .saturating_add(receipt.service_identity.len())
        .saturating_add(receipt.request_kind.len())
}
