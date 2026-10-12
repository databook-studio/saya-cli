//! Encoding durable records as journal lines: receipts, session markers,
//! and the incomplete marker.

use super::JournalReceipt;
use super::{GitOperation, GitProcessEffect, GitProcessOutcome, GitProcessReceipt, VERSION};

pub(in crate::coding::git::process_journal) const INCOMPLETE_LINE: &[u8] =
    b"{\"kind\":\"incomplete\",\"reason\":\"bounded\",\"version\":1}\n";

pub(in crate::coding::git) fn encode_receipt(
    receipt: &GitProcessReceipt,
) -> Result<Vec<u8>, serde_json::Error> {
    let receipt = JournalReceipt::from(receipt);
    let value = serde_json::json!({
        "kind":"receipt", "version":receipt.version,
        "service_identity":receipt.service_identity, "sequence":receipt.sequence,
        "request_kind":receipt.request_kind, "operation":operation_name(receipt.operation),
        "pid":receipt.pid, "exit_code":receipt.exit_code, "signal":receipt.signal,
        "outcome":outcome_name(receipt.outcome), "effect":effect_name(receipt.effect),
        "killed_orphans":receipt.killed_orphans, "duration_ms":receipt.duration_ms,
        "stdout_bytes":receipt.stdout_bytes, "stderr_bytes":receipt.stderr_bytes,
        "stdout_truncated":receipt.stdout_truncated, "stderr_truncated":receipt.stderr_truncated
    });
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(in crate::coding::git::process_journal) fn encode_session_start(
    service_identity: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    encode_session("session_start", service_identity)
}

pub(in crate::coding::git::process_journal) fn encode_session_close(
    service_identity: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    encode_session("session_close", service_identity)
}

fn encode_session(kind: &str, service_identity: &str) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec(&serde_json::json!({
        "kind": kind,
        "version": VERSION,
        "service_identity": service_identity,
    }))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn operation_name(operation: GitOperation) -> &'static str {
    match operation {
        GitOperation::Read => "read",
        GitOperation::Materialize => "materialize",
        GitOperation::Remove => "remove",
    }
}
fn outcome_name(outcome: GitProcessOutcome) -> &'static str {
    match outcome {
        GitProcessOutcome::Exited => "exited",
        GitProcessOutcome::TimedOut => "timed_out",
        GitProcessOutcome::Cancelled => "cancelled",
        GitProcessOutcome::SpawnFailed => "spawn_failed",
        GitProcessOutcome::WaitFailed => "wait_failed",
        GitProcessOutcome::CaptureFailed => "capture_failed",
        GitProcessOutcome::OutputLimited => "output_limited",
        GitProcessOutcome::Unknown => "unknown",
    }
}
fn effect_name(effect: GitProcessEffect) -> &'static str {
    match effect {
        GitProcessEffect::None => "none",
        GitProcessEffect::ProcessSucceeded => "process_succeeded",
        GitProcessEffect::Unknown => "unknown",
    }
}

impl From<&GitProcessReceipt> for JournalReceipt {
    fn from(receipt: &GitProcessReceipt) -> Self {
        Self {
            version: receipt.version,
            service_identity: receipt.service_identity.clone(),
            sequence: receipt.sequence,
            request_kind: receipt.request_kind,
            operation: receipt.operation,
            pid: receipt.pid,
            exit_code: receipt.exit_code,
            signal: receipt.signal,
            outcome: receipt.outcome,
            effect: receipt.effect,
            killed_orphans: receipt.killed_orphans,
            duration_ms: receipt.duration_ms,
            stdout_bytes: receipt.stdout_bytes,
            stderr_bytes: receipt.stderr_bytes,
            stdout_truncated: receipt.stdout_truncated,
            stderr_truncated: receipt.stderr_truncated,
        }
    }
}
