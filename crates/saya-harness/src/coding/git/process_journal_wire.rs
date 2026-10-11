use serde::Deserialize;

use super::super::{
    command::GitOperation,
    process_observer::{GitProcessEffect, GitProcessOutcome, GitProcessReceipt},
};

const VERSION: u32 = 1;
const MAX_LINE_BYTES: usize = 512;
const MAX_IDENTITY_BYTES: usize = 128;
const MAX_DURATION_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_STREAM_BYTES: u64 = 16 * 1024 * 1024;

fn request_kind(value: &str) -> Option<&'static str> {
    Some(match value {
        "version" => "version",
        "is_bare" => "is_bare",
        "repository_root" => "repository_root",
        "git_dir" => "git_dir",
        "common_dir" => "common_dir",
        "object_format" => "object_format",
        "local_config" => "local_config",
        "verify_commit" => "verify_commit",
        "verify_commits" => "verify_commits",
        "check_filter_attributes" => "check_filter_attributes",
        "initialize_index" => "initialize_index",
        "index_entries" => "index_entries",
        "index_stage_entries" => "index_stage_entries",
        "tree_entries" => "tree_entries",
        "read_blob" => "read_blob",
        "snapshot_blob" => "snapshot_blob",
        "head" => "head",
        "branch" => "branch",
        "unstaged_names" => "unstaged_names",
        "staged_names" => "staged_names",
        "status" => "status",
        "worktree_list" => "worktree_list",
        "status_branch" => "status_branch",
        "add_worktree" => "add_worktree",
        "add_worktree_no_checkout" => "add_worktree_no_checkout",
        "remove_worktree" => "remove_worktree",
        _ => return None,
    })
}

pub(super) struct JournalReceipt {
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireReceipt {
    kind: String,
    version: u32,
    service_identity: String,
    sequence: u64,
    request_kind: String,
    operation: String,
    pid: Option<u32>,
    exit_code: Option<i32>,
    signal: Option<i32>,
    outcome: String,
    effect: String,
    killed_orphans: Option<bool>,
    duration_ms: u64,
    stdout_bytes: u64,
    stderr_bytes: u64,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIncomplete {
    kind: String,
    reason: String,
    version: u32,
}

pub(super) fn decode_line(line: &[u8]) -> Option<Result<JournalReceipt, ()>> {
    if let Ok(marker) = serde_json::from_slice::<WireIncomplete>(line) {
        let _valid =
            marker.kind == "incomplete" && marker.reason == "bounded" && marker.version == VERSION;
        return Some(Err(()));
    }
    Some(
        serde_json::from_slice::<WireReceipt>(line)
            .map_err(|_| ())
            .and_then(|wire| validate_wire(wire).ok_or(())),
    )
}

pub(super) fn encode_receipt(receipt: &GitProcessReceipt) -> Result<Vec<u8>, serde_json::Error> {
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

pub(super) const INCOMPLETE_LINE: &[u8] =
    b"{\"kind\":\"incomplete\",\"reason\":\"bounded\",\"version\":1}\n";
pub(super) const MAX_LINE: usize = MAX_LINE_BYTES;

fn validate_wire(wire: WireReceipt) -> Option<JournalReceipt> {
    if wire.kind != "receipt"
        || wire.version != VERSION
        || wire.service_identity.is_empty()
        || wire.service_identity.len() > MAX_IDENTITY_BYTES
        || !wire
            .service_identity
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || wire.sequence == 0
        || wire.duration_ms > MAX_DURATION_MS
        || wire.stdout_bytes > MAX_STREAM_BYTES
        || wire.stderr_bytes > MAX_STREAM_BYTES
    {
        return None;
    }
    let request_kind = request_kind(&wire.request_kind)?;
    let operation = match wire.operation.as_str() {
        "read" => GitOperation::Read,
        "materialize" => GitOperation::Materialize,
        "remove" => GitOperation::Remove,
        _ => return None,
    };
    let outcome = match wire.outcome.as_str() {
        "exited" => GitProcessOutcome::Exited,
        "timed_out" => GitProcessOutcome::TimedOut,
        "cancelled" => GitProcessOutcome::Cancelled,
        "spawn_failed" => GitProcessOutcome::SpawnFailed,
        "wait_failed" => GitProcessOutcome::WaitFailed,
        "capture_failed" => GitProcessOutcome::CaptureFailed,
        "output_limited" => GitProcessOutcome::OutputLimited,
        "unknown" => GitProcessOutcome::Unknown,
        _ => return None,
    };
    let effect = match wire.effect.as_str() {
        "none" => GitProcessEffect::None,
        "process_succeeded" => GitProcessEffect::ProcessSucceeded,
        "unknown" => GitProcessEffect::Unknown,
        _ => return None,
    };
    if matches!(operation, GitOperation::Read) != matches!(effect, GitProcessEffect::None)
        || (matches!(effect, GitProcessEffect::ProcessSucceeded)
            && (!matches!(operation, GitOperation::Materialize | GitOperation::Remove)
                || outcome != GitProcessOutcome::Exited
                || wire.exit_code != Some(0)))
    {
        return None;
    }
    let exit_shape_valid = match outcome {
        GitProcessOutcome::Exited => {
            wire.pid.is_some()
                && ((wire.exit_code.is_some() && wire.signal.is_none())
                    || (wire.exit_code.is_none()
                        && wire
                            .signal
                            .is_some_and(|signal| (1..=128).contains(&signal))))
        }
        _ => wire.exit_code.is_none() && wire.signal.is_none(),
    };
    if !exit_shape_valid {
        return None;
    }
    Some(JournalReceipt {
        version: wire.version,
        service_identity: wire.service_identity,
        sequence: wire.sequence,
        request_kind,
        operation,
        pid: wire.pid,
        exit_code: wire.exit_code,
        signal: wire.signal,
        outcome,
        effect,
        killed_orphans: wire.killed_orphans,
        duration_ms: wire.duration_ms,
        stdout_bytes: wire.stdout_bytes,
        stderr_bytes: wire.stderr_bytes,
        stdout_truncated: wire.stdout_truncated,
        stderr_truncated: wire.stderr_truncated,
    })
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

impl From<JournalReceipt> for GitProcessReceipt {
    fn from(receipt: JournalReceipt) -> Self {
        Self {
            version: receipt.version,
            service_identity: receipt.service_identity,
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
