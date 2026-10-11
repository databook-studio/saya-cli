use std::collections::HashMap;
use std::fmt;

use saya_types::Sha256Digest;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::coding::snapshot::{MAX_SNAPSHOT_MANIFEST_BYTES, SnapshotPath};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlanError {
    #[error("the stored plan manifest is malformed")]
    Malformed,
    #[error("the stored plan manifest holds an unsafe path")]
    UnsafePath,
    #[error("the stored plan bytes do not match their digest")]
    DigestMismatch,
    #[error("the stored plan manifest exceeds its bound")]
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanIndexEntry {
    pub mode: String,
    pub object_id: String,
    pub stage: u8,
    pub intent_to_add: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanStatusFact {
    pub staged: bool,
    pub unstaged: bool,
    pub staged_added: bool,
    pub unstaged_added: bool,
    pub staged_deleted: bool,
    pub unstaged_deleted: bool,
    pub untracked: bool,
    pub ignored: bool,
}

pub struct PlanIndex {
    digest: Sha256Digest,
    index: HashMap<String, PlanIndexEntry>,
    status: HashMap<String, PlanStatusFact>,
}

impl PlanIndex {
    pub fn parse(plan_sha256: &Sha256Digest, plan_bytes: &[u8]) -> Result<Self, PlanError> {
        if plan_bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
            return Err(PlanError::TooLarge);
        }
        let computed = plan_digest(plan_bytes)?;
        if computed != *plan_sha256 {
            return Err(PlanError::DigestMismatch);
        }
        let body: PlanManifestBody =
            serde_json::from_slice(plan_bytes).map_err(|_| PlanError::Malformed)?;
        let mut index = HashMap::with_capacity(body.index_entries.len());
        for entry in body.index_entries {
            validate_path(&entry.path)?;
            index.insert(
                entry.path,
                PlanIndexEntry {
                    mode: entry.mode.to_string(),
                    object_id: entry.object_id,
                    stage: entry.stage,
                    intent_to_add: entry.intent_to_add,
                },
            );
        }
        let mut status = HashMap::with_capacity(body.status.len());
        for fact in body.status {
            validate_path(&fact.path)?;
            status.insert(
                fact.path,
                PlanStatusFact {
                    staged: fact.staged,
                    unstaged: fact.unstaged,
                    staged_added: fact.staged_added,
                    unstaged_added: fact.unstaged_added,
                    staged_deleted: fact.staged_deleted,
                    unstaged_deleted: fact.unstaged_deleted,
                    untracked: fact.untracked,
                    ignored: fact.ignored,
                },
            );
        }
        Ok(Self {
            digest: computed,
            index,
            status,
        })
    }

    pub fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    pub fn index_entry(&self, path: &str) -> Option<&PlanIndexEntry> {
        self.index.get(path)
    }

    pub fn status_fact(&self, path: &str) -> Option<&PlanStatusFact> {
        self.status.get(path)
    }
}

impl fmt::Debug for PlanIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlanIndex")
            .field("index_entry_count", &self.index.len())
            .field("status_fact_count", &self.status.len())
            .finish()
    }
}

fn plan_digest(plan_bytes: &[u8]) -> Result<Sha256Digest, PlanError> {
    let digest = Sha256::digest(plan_bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").map_err(|_| PlanError::Malformed)?;
    }
    Sha256Digest::parse(&hex).map_err(|_| PlanError::Malformed)
}

fn validate_path(path: &str) -> Result<(), PlanError> {
    SnapshotPath::parse(path.to_owned()).map_err(|_| PlanError::UnsafePath)
}

#[derive(Deserialize)]
struct PlanManifestBody {
    index_entries: Vec<PlanIndexEntryMirror>,
    status: Vec<PlanStatusFactMirror>,
}

#[derive(Deserialize)]
struct PlanIndexEntryMirror {
    path: String,
    mode: u32,
    object_id: String,
    stage: u8,
    intent_to_add: bool,
}

#[derive(Deserialize)]
struct PlanStatusFactMirror {
    path: String,
    staged: bool,
    unstaged: bool,
    staged_added: bool,
    unstaged_added: bool,
    staged_deleted: bool,
    unstaged_deleted: bool,
    untracked: bool,
    ignored: bool,
}
