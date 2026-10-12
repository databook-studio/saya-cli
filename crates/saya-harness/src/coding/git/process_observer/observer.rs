//! The live session: sequence assignment, bounded retention, and journal hand-off.

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use super::super::command::{GitError, GitOperation};
use super::super::process::GitOutput;
use super::super::process_journal::{GitProcessJournal, JournalAppendError, JournalState};
use super::receipt::{GitProcessOutcome, GitProcessReceipt, RECEIPT_VERSION, effect, outcome};
use super::snapshot::GitProcessSnapshot;

const MAX_RECEIPTS: usize = 256;
const MAX_RECEIPT_BYTES: usize = 512;
const MAX_JOURNAL_BYTES: usize = 16 * 1024;

pub(in crate::coding::git) struct GitProcessObserver {
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
    pub(in crate::coding::git) fn new(
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

    pub(in crate::coding::git) fn begin(
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

    pub(in crate::coding::git) fn snapshot(&self) -> GitProcessSnapshot {
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

    pub(in crate::coding::git) fn close(&self) {
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

pub(in crate::coding::git) struct Observation {
    observer: Arc<GitProcessObserver>,
    receipt: GitProcessReceipt,
    started: Instant,
    finished: bool,
}

impl Observation {
    pub(in crate::coding::git) fn pid(&mut self, pid: u32) {
        self.receipt.pid = Some(pid);
    }
    pub(in crate::coding::git) fn settled(&mut self, duration_ms: u64, killed_orphans: bool) {
        self.receipt.duration_ms = duration_ms;
        self.receipt.killed_orphans = Some(killed_orphans);
    }
    pub(in crate::coding::git) fn capture(
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
    pub(in crate::coding::git) fn finish(&mut self, result: &Result<GitOutput, GitError>) {
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

fn receipt_bytes(receipt: &GitProcessReceipt) -> usize {
    128usize
        .saturating_add(receipt.service_identity.len())
        .saturating_add(receipt.request_kind.len())
}
