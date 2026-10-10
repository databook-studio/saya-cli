use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
};

use saya_agent::CancellationToken;
use saya_types::GitObjectId;
use tokio::time::Instant;

use super::{
    command::{
        GitConfigRisk, GitError, GitObjectFormat, GitOperation, GitRepository, require_success,
    },
    index_parser::GitIndexEntry,
    inventory::InventoryRead,
    inventory_deadline::InventoryDeadline,
    inventory_types::{GitPathStatus, GitRepositoryInventory},
    parser::{InventoryBudget, MAX_SNAPSHOT_INPUT_PATHS},
    repository::{GitHeadState, GitInventoryError, GitRepositoryIdentity},
    request::GitRequest,
    service::GitService,
    snapshot_tree_parser::{GitTreeEntry, parse_tree_entries, reject_git_components},
};

pub(crate) struct SnapshotIndexInput {
    pub(crate) path: PathBuf,
    pub(crate) mode: u32,
    pub(crate) object_id: GitObjectId,
    pub(crate) stage: u8,
    pub(crate) intent_to_add: bool,
}

pub(crate) struct SnapshotTreeInput {
    pub(crate) path: PathBuf,
    pub(crate) mode: u32,
    pub(crate) object_id: GitObjectId,
    pub(crate) declared_bytes: u64,
}

pub(crate) struct GitSnapshotInputs {
    pub(crate) identity: GitRepositoryIdentity,
    pub(crate) head: GitHeadState,
    pub(crate) status: Vec<GitPathStatus>,
    pub(crate) index_entries: Vec<SnapshotIndexInput>,
    pub(crate) tree_entries: Option<Vec<SnapshotTreeInput>>,
}

impl GitService {
    pub(crate) async fn read_snapshot_inputs_until(
        &self,
        repository: &GitRepository,
        selection: crate::coding::snapshot::SnapshotSelection,
        cancellation: &CancellationToken,
        deadline_at: Instant,
    ) -> Result<GitSnapshotInputs, GitInventoryError> {
        let _guard = self
            .operation_guard_until(cancellation, deadline_at)
            .await
            .map_err(GitInventoryError::Git)?;
        let deadline = InventoryDeadline::until(deadline_at);
        deadline.check()?;
        let InventoryRead {
            inventory,
            indexed_entries,
            mut budget,
        } = self
            .inventory_locked(
                repository,
                cancellation,
                deadline,
                InventoryBudget::with_path_limit(MAX_SNAPSHOT_INPUT_PATHS),
            )
            .await?;
        let base = match &inventory.head {
            GitHeadState::Commit(object) => Some(object),
            GitHeadState::Unborn => None,
        };
        let tree_entries = if selection == crate::coding::snapshot::SnapshotSelection::Head {
            if let Some(base) = base {
                let output = self
                    .run_request_until(
                        repository.root(),
                        GitRequest::TreeEntries(base),
                        cancellation,
                        deadline_at,
                    )
                    .await
                    .map_err(GitInventoryError::Git)?;
                let output =
                    require_success(output, GitOperation::Read).map_err(GitInventoryError::Git)?;
                budget.add_output(&output.stdout)?;
                Some(parse_tree_entries(
                    &output.stdout,
                    repository.object_format(),
                    &mut budget,
                )?)
            } else {
                None
            }
        } else {
            None
        };
        deadline.check()?;
        let attribute_paths = attribute_paths(
            selection,
            &inventory,
            &indexed_entries,
            tree_entries.as_deref(),
            deadline,
            cancellation,
        )?;
        if !attribute_paths.is_empty() {
            let attribute_source = if selection == crate::coding::snapshot::SnapshotSelection::Head
            {
                base
            } else {
                None
            };
            let output = self
                .run_request_until(
                    repository.root(),
                    GitRequest::CheckFilterAttributes {
                        source: attribute_source,
                        paths: &attribute_paths,
                    },
                    cancellation,
                    deadline_at,
                )
                .await
                .map_err(GitInventoryError::Git)?;
            let output =
                require_success(output, GitOperation::Read).map_err(GitInventoryError::Git)?;
            budget.add_output(&output.stdout)?;
            reject_filter_attributes(&output.stdout, &attribute_paths)?;
        }
        deadline.check()?;
        let intent_to_add_paths: HashSet<&Path> = inventory
            .paths
            .iter()
            .filter(|status| status.unstaged_added)
            .map(|status| status.path.as_path())
            .collect();
        let mut index_entries = Vec::with_capacity(indexed_entries.len());
        for entry in indexed_entries {
            deadline.check()?;
            if cancellation.is_cancelled() {
                return Err(GitInventoryError::Git(GitError::Cancelled));
            }
            index_entries.push(snapshot_index(
                entry,
                repository.object_format(),
                &intent_to_add_paths,
            ));
        }
        Ok(GitSnapshotInputs {
            identity: inventory.identity,
            head: inventory.head,
            status: inventory.paths,
            index_entries,
            tree_entries: tree_entries
                .map(|entries| entries.into_iter().map(snapshot_tree).collect()),
        })
    }
}

fn attribute_paths(
    selection: crate::coding::snapshot::SnapshotSelection,
    inventory: &GitRepositoryInventory,
    index_entries: &[GitIndexEntry],
    tree_entries: Option<&[GitTreeEntry]>,
    deadline: InventoryDeadline,
    cancellation: &CancellationToken,
) -> Result<Vec<String>, GitInventoryError> {
    let mut paths = BTreeSet::new();
    let candidates = if selection == crate::coding::snapshot::SnapshotSelection::Head {
        tree_entries
            .ok_or(GitInventoryError::UnsupportedRepositoryState)?
            .iter()
            .map(|entry| &entry.path)
            .collect::<Vec<_>>()
    } else {
        index_entries
            .iter()
            .map(|entry| &entry.path)
            .chain(
                inventory
                    .paths
                    .iter()
                    .filter(|entry| !entry.ignored)
                    .map(|entry| &entry.path),
            )
            .collect::<Vec<_>>()
    };
    if candidates.len() > MAX_SNAPSHOT_INPUT_PATHS {
        return Err(GitInventoryError::TooLarge);
    }
    for path in candidates {
        deadline.check()?;
        if cancellation.is_cancelled() {
            return Err(GitInventoryError::Git(GitError::Cancelled));
        }
        reject_git_components(path)?;
        let path = path
            .to_str()
            .ok_or(GitInventoryError::UnsupportedRepositoryState)?;
        paths.insert(path.to_owned());
        if paths.len() > MAX_SNAPSHOT_INPUT_PATHS {
            return Err(GitInventoryError::TooLarge);
        }
    }
    Ok(paths.into_iter().collect())
}

fn reject_filter_attributes(
    bytes: &[u8],
    expected_paths: &[String],
) -> Result<(), GitInventoryError> {
    if !bytes.is_empty() && !bytes.ends_with(b"\0") {
        return Err(GitInventoryError::MalformedOutput);
    }
    let fields: Vec<_> = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect();
    if fields.len() != expected_paths.len() * 3 {
        return Err(GitInventoryError::MalformedOutput);
    }
    for (record, expected) in fields.chunks_exact(3).zip(expected_paths) {
        if record[0] != expected.as_bytes() || record[1] != b"filter" {
            return Err(GitInventoryError::MalformedOutput);
        }
        if record[2] != b"unspecified" && record[2] != b"unset" {
            return Err(GitInventoryError::Git(GitError::UnsupportedConfiguration(
                GitConfigRisk::Filter,
            )));
        }
    }
    Ok(())
}

fn snapshot_index(
    entry: GitIndexEntry,
    format: GitObjectFormat,
    intent_to_add_paths: &HashSet<&Path>,
) -> SnapshotIndexInput {
    let intent_to_add = entry.stage == 0
        && intent_to_add_paths.contains(entry.path.as_path())
        && entry.object_id.as_str() == empty_blob_id(format);
    SnapshotIndexInput {
        path: entry.path,
        mode: entry.mode,
        object_id: entry.object_id,
        stage: entry.stage,
        intent_to_add,
    }
}

/// Installed Git records `git add -N` as a stage-0 index entry whose object
/// ID is the empty blob for the repository's object format.
fn empty_blob_id(format: GitObjectFormat) -> &'static str {
    match format {
        GitObjectFormat::Sha1 => "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
        GitObjectFormat::Sha256 => {
            "473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813"
        }
    }
}

fn snapshot_tree(entry: GitTreeEntry) -> SnapshotTreeInput {
    SnapshotTreeInput {
        path: entry.path,
        mode: entry.mode,
        object_id: entry.object_id,
        declared_bytes: entry.declared_bytes,
    }
}
