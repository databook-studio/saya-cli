//! Decoding and validating one journal line into a `JournalLine` record.

use serde::Deserialize;

use super::{
    GitOperation, GitProcessEffect, GitProcessOutcome, GitProcessReceipt, JournalLine,
    JournalReceipt, VERSION,
};

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
struct WireSession {
    kind: String,
    version: u32,
    service_identity: String,
}

pub(in crate::coding::git) fn decode_line(line: &[u8]) -> Option<Result<JournalLine, ()>> {
    // A line shaped like the bounded incomplete marker (exactly the keys
    // kind, reason and version) makes the journal incomplete whatever its
    // values: a well-formed marker and a corrupt one are both refused here,
    // so the values are deliberately not interpreted.
    if let Ok(marker) =
        serde_json::from_slice::<std::collections::BTreeMap<String, serde_json::Value>>(line)
        && marker
            .keys()
            .map(String::as_str)
            .eq(["kind", "reason", "version"])
    {
        return Some(Err(()));
    }
    if let Ok(session) = serde_json::from_slice::<WireSession>(line) {
        if session.version != VERSION || !valid_identity(&session.service_identity) {
            return Some(Err(()));
        }
        return Some(match session.kind.as_str() {
            "session_start" => Ok(JournalLine::SessionStart(session.service_identity)),
            "session_close" => Ok(JournalLine::SessionClose(session.service_identity)),
            _ => Err(()),
        });
    }
    Some(
        serde_json::from_slice::<WireReceipt>(line)
            .map_err(|_| ())
            .and_then(|wire| validate_wire(wire).map(JournalLine::Receipt).ok_or(())),
    )
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTITY_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_wire(wire: WireReceipt) -> Option<JournalReceipt> {
    if wire.kind != "receipt"
        || wire.version != VERSION
        || !valid_identity(&wire.service_identity)
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
