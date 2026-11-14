//! Strict private metadata for bytes known committed before diagnostics run.

use super::{
    CodingError, KnownWorkspaceMutation, KnownWorkspaceMutationOperation,
    checks::ChildCodingOperations,
    context,
    rewind::{RewindStore, store_root},
};
use saya_harness::coding::integration::DeltaKind;
use saya_types::{ManagedCheckoutId, Sha256Digest};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path};

pub(super) const MAX_KNOWN_COMMIT_BYTES: usize = 1024 * 1024;

/// One committed mutation the journal records, latest record per path. Review
/// and rewind both consume this shape, so the two cannot disagree about what a
/// job committed.
pub(super) struct KnownCommit {
    operation: KnownWorkspaceMutationOperation,
    sha256: Sha256Digest,
}

pub(super) fn append(
    operations: &ChildCodingOperations,
    committed: &KnownWorkspaceMutation,
) -> Result<(), CodingError> {
    let turn = RewindStore::open(&store_root(&operations.records)).next_turn()?;
    let operation = match committed.operation() {
        KnownWorkspaceMutationOperation::Write => "write",
        KnownWorkspaceMutationOperation::Edit => "edit",
        KnownWorkspaceMutationOperation::Delete => "delete",
    };
    let path = committed.path().as_str();
    let value = json!({
        "schema_version": 1,
        "job_id": operations.job,
        "checkout_id": operations.checkout,
        "actor": {
            "agent_id": operations.authority.agent_id(),
            "invocation_id": operations.authority.invocation_id(),
        },
        "operation": operation,
        "path": path,
        "path_sha256": super::create::digest(path.as_bytes())?,
        "committed_sha256": committed.sha256(),
        "size": committed.size(),
        "turn": turn,
    });
    append_record(
        &journal_path(&operations.records, &operations.checkout),
        &value,
    )
}

fn journal_path(records: &Path, checkout: &ManagedCheckoutId) -> std::path::PathBuf {
    records.join(format!("known-commits-{}.jsonl", checkout.as_str()))
}

fn append_record(path: &Path, value: &Value) -> Result<(), CodingError> {
    context::append_line(
        path,
        &serde_json::to_vec(value).map_err(CodingError::owner)?,
        MAX_KNOWN_COMMIT_BYTES,
    )
}

/// The job's journal, latest record per path, or `None` when no journal
/// exists — so review and rewind state no basis they do not have. A malformed
/// record is a refusal, never a silently weaker basis.
pub(super) fn read(
    records: &Path,
    checkout: &ManagedCheckoutId,
) -> Result<Option<HashMap<String, KnownCommit>>, CodingError> {
    let path = journal_path(records, checkout);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = context::read(&path, MAX_KNOWN_COMMIT_BYTES)?;
    let mut known = HashMap::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: Value = serde_json::from_slice(line).map_err(CodingError::owner)?;
        let object = value.as_object().ok_or(CodingError::InvalidRequest)?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or(CodingError::InvalidRequest)?;
        let operation = match object.get("operation").and_then(Value::as_str) {
            Some("write") => KnownWorkspaceMutationOperation::Write,
            Some("edit") => KnownWorkspaceMutationOperation::Edit,
            Some("delete") => KnownWorkspaceMutationOperation::Delete,
            _ => return Err(CodingError::InvalidRequest),
        };
        let digest = object
            .get("committed_sha256")
            .and_then(Value::as_str)
            .ok_or(CodingError::InvalidRequest)?;
        let sha256 = Sha256Digest::parse(digest).map_err(CodingError::owner)?;
        if object.get("turn").and_then(Value::as_u64).is_none() {
            return Err(CodingError::InvalidRequest);
        }
        known.insert(path.to_owned(), KnownCommit { operation, sha256 });
    }
    Ok(Some(known))
}

/// Whether one current change is not accounted for by the job's own journal.
/// A path the journal never names is foreign; so is a write the journal holds
/// at another digest, and a removal the journal does not record. A deleted
/// path the journal records as deleted is the job's own removal. Without a
/// journal there is no basis, so nothing is called foreign.
pub(super) fn is_foreign(
    known: Option<&HashMap<String, KnownCommit>>,
    path: &str,
    kind: DeltaKind,
    content_sha256: Option<&Sha256Digest>,
) -> bool {
    let Some(known) = known else {
        return false;
    };
    let Some(commit) = known.get(path) else {
        return true;
    };
    match (kind, commit.operation) {
        (DeltaKind::Deleted, KnownWorkspaceMutationOperation::Delete) => false,
        (DeltaKind::Deleted, _) => true,
        (_, KnownWorkspaceMutationOperation::Delete) => true,
        (_, _) => content_sha256.is_none_or(|digest| digest != &commit.sha256),
    }
}

/// Maps private durable receipt records to the only safe export projection.
/// Invalid or legacy records remain an export failure, never a best-effort
/// alias that could misstate a committed path.
pub(super) fn export_records(bytes: &[u8]) -> Result<Vec<Value>, CodingError> {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(export_record)
        .collect()
}

fn export_record(bytes: &[u8]) -> Result<Value, CodingError> {
    let value: Value = serde_json::from_slice(bytes).map_err(CodingError::owner)?;
    let object = value.as_object().ok_or(CodingError::InvalidRequest)?;
    let schema = object
        .get("schema_version")
        .and_then(Value::as_u64)
        .filter(|version| *version == 1)
        .ok_or(CodingError::InvalidRequest)?;
    let operation = object
        .get("operation")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "write" | "edit" | "delete"))
        .ok_or(CodingError::InvalidRequest)?;
    let path = string_field(object, "path")?;
    let path: saya_types::WorkspacePath = path
        .to_owned()
        .try_into()
        .map_err(|_| CodingError::InvalidRequest)?;
    let path_sha256 = digest_field(object, "path_sha256")?;
    if super::create::digest(path.as_str().as_bytes())?.as_str() != path_sha256 {
        return Err(CodingError::InvalidRequest);
    }
    let committed_sha256 = digest_field(object, "committed_sha256")?;
    let size = object
        .get("size")
        .and_then(Value::as_u64)
        .ok_or(CodingError::InvalidRequest)?;
    let actor = object
        .get("actor")
        .and_then(Value::as_object)
        .ok_or(CodingError::InvalidRequest)?;
    let agent_id = string_field(actor, "agent_id")?;
    let invocation_id = string_field(actor, "invocation_id")?;
    let job_id = string_field(object, "job_id")?;
    let checkout_id = string_field(object, "checkout_id")?;
    saya_types::JobId::parse(job_id).map_err(CodingError::owner)?;
    saya_types::ManagedCheckoutId::parse(checkout_id).map_err(CodingError::owner)?;
    saya_types::AgentId::parse(agent_id).map_err(CodingError::owner)?;
    saya_types::InvocationId::parse(invocation_id).map_err(CodingError::owner)?;
    Ok(json!({
        "schema_version": schema,
        "job_id": job_id,
        "checkout_id": checkout_id,
        "actor": {"agent_id":agent_id,"invocation_id":invocation_id},
        "operation": operation,
        "path": {"projection":"private_relative_path_sha256","sha256":path_sha256},
        "committed_sha256": committed_sha256,
        "size": size,
    }))
}

fn string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, CodingError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(CodingError::InvalidRequest)
}

fn digest_field(object: &serde_json::Map<String, Value>, key: &str) -> Result<String, CodingError> {
    let value = string_field(object, key)?;
    saya_types::Sha256Digest::parse(value).map_err(CodingError::owner)?;
    Ok(value.to_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn journal_bounded() {
        let dir = std::env::temp_dir().join(format!(
            "saya-known-commit-bound-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("known-commits-journal.jsonl");
        // One byte short of the bound: any record at all crosses it.
        std::fs::write(&path, vec![b'\n'; MAX_KNOWN_COMMIT_BYTES - 1]).unwrap();
        let record = json!({"schema_version":1,"path":"src/lib.rs","turn":1});
        assert!(
            append_record(&path, &record).is_err(),
            "a record that would pass the journal's byte bound is refused"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            (MAX_KNOWN_COMMIT_BYTES - 1) as u64,
            "a refused append writes nothing"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_projection_keeps_identity_fingerprint_but_not_private_path() {
        let path = "src/private.rs";
        let path_sha256 = super::super::create::digest(path.as_bytes()).unwrap();
        let record = json!({"schema_version":1,"job_id":"job-a","checkout_id":"checkout-a","actor":{"agent_id":"agent-a","invocation_id":"invocation-a"},"operation":"write","path":path,"path_sha256":path_sha256.clone(),"committed_sha256":"b".repeat(64),"size":12});
        let exported = export_records(format!("{record}\n").as_bytes()).unwrap();
        assert_eq!(
            exported[0]["path"]["projection"],
            "private_relative_path_sha256"
        );
        assert_eq!(exported[0]["path"]["sha256"], json!(path_sha256));
        assert!(!exported[0].to_string().contains(path));
    }
}
