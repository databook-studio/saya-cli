use std::collections::BTreeSet;

use saya_agent::CancellationToken;
use saya_types::{GitObjectId, SnapshotId};
use tokio::time::{Duration, Instant};

use crate::{
    coding::git::{GitHeadState, GitRepository, GitService},
    workspace::Workspace,
};

use super::{
    facts::{SnapshotChange, SnapshotStatusFact},
    manifest::{SnapshotError, SnapshotManifest, SnapshotManifestBody, digest, manifest_bytes},
    policy::SnapshotOmissionPolicy,
    select_support::{check_control, file_mode, path_string, sensitive_path, snapshot_status_path},
    symlink::{SymlinkSet, validate_symlink_target},
    types::*,
};
#[cfg(unix)]
use super::{manifest::RedactedIoError, select_support::status_proves_deletion};

const SNAPSHOT_DEADLINE: Duration = Duration::from_secs(30);

/// One selection candidate: the workspace-relative path plus the git entry
/// it came from when the path is tracked.
type SelectionCandidate = (String, Option<(GitObjectId, u32, u64)>);

impl GitService {
    pub async fn plan_snapshot(
        &self,
        repository: &GitRepository,
        snapshot_id: SnapshotId,
        selection: SnapshotSelection,
        policy: SnapshotOmissionPolicy,
        cancellation: &CancellationToken,
    ) -> Result<SnapshotManifest, SnapshotError> {
        if policy.root != repository.root() || policy.common_dir != repository.common_dir() {
            return Err(SnapshotError::PolicyRepositoryMismatch);
        }
        let deadline = Instant::now() + SNAPSHOT_DEADLINE;
        let inputs = self
            .read_snapshot_inputs_until(repository, selection, cancellation, deadline)
            .await?;
        check_control(cancellation, deadline)?;
        let base_commit = match inputs.head {
            GitHeadState::Commit(commit) => commit,
            GitHeadState::Unborn => return Err(SnapshotError::UnbornHead),
        };
        if inputs.status.iter().any(|status| status.conflicted())
            || inputs.index_entries.iter().any(|entry| entry.stage != 0)
        {
            return Err(SnapshotError::ConflictedIndex);
        }
        let identity = inputs.identity;
        let tracked: BTreeSet<String> = inputs
            .index_entries
            .iter()
            .map(|entry| path_string(&entry.path))
            .collect::<Result<_, _>>()?;
        let mut index_entries = Vec::with_capacity(inputs.index_entries.len());
        for entry in inputs.index_entries {
            index_entries.push(SnapshotIndexEntry {
                path: SnapshotPath::parse(path_string(&entry.path)?)?,
                mode: entry.mode,
                object_id: entry.object_id,
                stage: entry.stage,
                intent_to_add: entry.intent_to_add,
            });
        }
        let index_bytes =
            serde_json::to_vec(&index_entries).map_err(|_| SnapshotError::Serialization)?;
        if index_bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
            return Err(SnapshotError::ManifestBytesExceeded {
                measured: index_bytes.len(),
                limit: MAX_SNAPSHOT_MANIFEST_BYTES,
            });
        }
        let index_sha256 = digest(&index_bytes)?;
        let mut status_facts = Vec::new();
        let mut changes = Vec::new();
        let mut omissions = Vec::new();
        let mut warnings = Vec::new();
        for status in &inputs.status {
            let path = snapshot_status_path(status.path())?;
            let parsed = SnapshotPath::parse(path.clone())?;
            status_facts.push(SnapshotStatusFact {
                path: parsed.clone(),
                original_path: status
                    .original_path()
                    .map(snapshot_status_path)
                    .transpose()?
                    .map(SnapshotPath::parse)
                    .transpose()?,
                staged: status.staged(),
                unstaged: status.unstaged(),
                staged_added: status.staged_added(),
                unstaged_added: status.unstaged_added(),
                staged_deleted: status.staged_deleted(),
                unstaged_deleted: status.unstaged_deleted(),
                untracked: status.untracked(),
                ignored: status.ignored(),
            });
            if status.ignored() {
                omissions.push(SnapshotOmissionFact {
                    path: parsed.clone(),
                    reason: SnapshotOmissionReason::Ignored,
                });
            }
            if let Some(from) = status.original_path() {
                changes.push(SnapshotChange::Rename {
                    from: SnapshotPath::parse(snapshot_status_path(from)?)?,
                    to: parsed.clone(),
                    staged: status.staged(),
                    unstaged: status.unstaged(),
                });
            }
            if status.staged_deleted() || status.unstaged_deleted() {
                changes.push(SnapshotChange::Tombstone { path: parsed });
            }
        }
        let tree = inputs.tree_entries.unwrap_or_default();
        let candidates: Vec<SelectionCandidate> = match selection {
            SnapshotSelection::Head => tree
                .into_iter()
                .map(|item| {
                    Ok((
                        path_string(&item.path)?,
                        Some((item.object_id, item.mode, item.declared_bytes)),
                    ))
                })
                .collect::<Result<_, SnapshotError>>()?,
            SnapshotSelection::CurrentWorkingContent => {
                let mut paths: BTreeSet<String> = tracked.clone();
                for status in &inputs.status {
                    if !status.ignored() {
                        paths.insert(snapshot_status_path(status.path())?);
                    }
                }
                paths.into_iter().map(|path| (path, None)).collect()
            }
        };
        let workspace =
            Workspace::open(repository.root()).map_err(SnapshotError::from_workspace_error)?;
        let mut entries = Vec::new();
        let mut links = SymlinkSet::default();
        let mut total = 0u64;
        for (path, head) in candidates {
            check_control(cancellation, deadline)?;
            let path_value = SnapshotPath::parse(path.clone())?;
            let is_tracked = head.is_some() || tracked.contains(&path);
            let private = policy
                .private_prefixes
                .iter()
                .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")));
            let sensitive = sensitive_path(&path);
            if is_tracked {
                // An explicit private prefix keeps the path out of the
                // snapshot. Otherwise tracked credential-shaped content is
                // repository content: include it and state a warning naming
                // the path, never its bytes.
                if private {
                    omissions.push(SnapshotOmissionFact {
                        path: path_value,
                        reason: SnapshotOmissionReason::ConfiguredPrivatePath,
                    });
                    continue;
                }
                if sensitive {
                    warnings.push(SnapshotWarningFact {
                        path: path_value.clone(),
                        reason: SnapshotWarningReason::SensitiveTrackedIncluded,
                    });
                }
            } else if private || sensitive {
                omissions.push(SnapshotOmissionFact {
                    path: path_value,
                    reason: if private {
                        SnapshotOmissionReason::ConfiguredPrivatePath
                    } else {
                        SnapshotOmissionReason::SensitiveUntracked
                    },
                });
                continue;
            }
            let (source, mode, size, source_identity) = if let Some((object, mode, size)) = head {
                if mode == 0o120000 {
                    if size > MAX_SYMLINK_TARGET_BYTES as u64 {
                        return Err(SnapshotError::unsafe_symlink(
                            &path,
                            "the target is longer than 4096 bytes",
                        ));
                    }
                    let target = self
                        .read_snapshot_blob(repository, &object, cancellation, deadline)
                        .await?;
                    if target.len() as u64 != size {
                        return Err(SnapshotError::BlobSizeMismatch);
                    }
                    validate_symlink_target(&path, &target)?;
                    links.insert(&path, target);
                    (
                        SnapshotContentSource::GitBlob { object_id: object },
                        SnapshotFileMode::Symlink,
                        size,
                        None,
                    )
                } else if mode == 0o100644 || mode == 0o100755 {
                    (
                        SnapshotContentSource::GitBlob { object_id: object },
                        file_mode(mode),
                        size,
                        None,
                    )
                } else {
                    return Err(SnapshotError::UnsupportedFileType);
                }
            } else {
                #[cfg(unix)]
                {
                    let anchor = match workspace.anchor_link(&path) {
                        Ok(anchor) => anchor,
                        Err(crate::HarnessError::NotFound { .. })
                            if status_proves_deletion(&inputs.status, &path) =>
                        {
                            continue;
                        }
                        Err(crate::HarnessError::NotFound { .. }) => {
                            return Err(SnapshotError::unsafe_path());
                        }
                        Err(error) => return Err(SnapshotError::from_workspace_error(error)),
                    };
                    let stat = match anchor.stat() {
                        Some(stat) => stat,
                        None if status_proves_deletion(&inputs.status, &path) => continue,
                        None => return Err(SnapshotError::unsafe_path()),
                    };
                    let (device, inode) = stat.identity();
                    if stat.is_symlink() {
                        let target =
                            anchor
                                .read_link(MAX_SYMLINK_TARGET_BYTES)
                                .map_err(|error| {
                                    if error.kind() == std::io::ErrorKind::InvalidData {
                                        SnapshotError::unsafe_symlink(
                                            &path,
                                            "the target is longer than 4096 bytes",
                                        )
                                    } else {
                                        SnapshotError::CaptureIo {
                                            description: "read the snapshot symlink".to_owned(),
                                            source: Some(RedactedIoError::new(error)),
                                        }
                                    }
                                })?;
                        validate_symlink_target(&path, &target)?;
                        let size = target.len() as u64;
                        links.insert(&path, target);
                        (
                            SnapshotContentSource::WorkingTree,
                            SnapshotFileMode::Symlink,
                            size,
                            Some(SnapshotSourceIdentity::new(device, inode, size, 0o120000)),
                        )
                    } else if stat.is_file() {
                        (
                            SnapshotContentSource::WorkingTree,
                            if stat.has_exec_bits() {
                                SnapshotFileMode::Executable
                            } else {
                                SnapshotFileMode::Regular
                            },
                            stat.len(),
                            Some(SnapshotSourceIdentity::new(
                                device,
                                inode,
                                stat.len(),
                                if stat.has_exec_bits() {
                                    0o100755
                                } else {
                                    0o100644
                                },
                            )),
                        )
                    } else {
                        return Err(SnapshotError::UnsupportedFileType);
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = &workspace;
                    return Err(SnapshotError::JobsUnsupportedOnWindows);
                }
            };
            if size > MAX_SNAPSHOT_FILE_BYTES {
                return Err(SnapshotError::FileBytesExceeded {
                    path: path_value.as_str().to_owned(),
                    measured: size,
                    limit: MAX_SNAPSHOT_FILE_BYTES,
                });
            }
            let measured = total.saturating_add(size);
            if measured > MAX_SNAPSHOT_TOTAL_BYTES {
                return Err(SnapshotError::TotalBytesExceeded {
                    measured,
                    limit: MAX_SNAPSHOT_TOTAL_BYTES,
                });
            }
            total = measured;
            entries.push(SnapshotFileEntry {
                path: path_value,
                source,
                mode,
                declared_bytes: size,
                source_identity,
            });
            if entries.len() > MAX_SNAPSHOT_FILES {
                return Err(SnapshotError::FileCountExceeded {
                    measured: entries.len(),
                    limit: MAX_SNAPSHOT_FILES,
                });
            }
        }
        links
            .validate()
            .map_err(|refusal| SnapshotError::unsafe_symlink(&refusal.path, refusal.reason))?;
        entries.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
        status_facts.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
        changes.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        omissions.sort_by(|a, b| {
            (a.path.as_str(), a.reason as u8).cmp(&(b.path.as_str(), b.reason as u8))
        });
        warnings.sort_by(|a, b| {
            (a.path.as_str(), a.reason as u8).cmp(&(b.path.as_str(), b.reason as u8))
        });
        // One durable event per fact is the reason for the cap: a repository
        // full of credential-shaped files must not flood the job's stream.
        let warnings_truncated = warnings.len().saturating_sub(MAX_SNAPSHOT_WARNINGS) as u64;
        warnings.truncate(MAX_SNAPSHOT_WARNINGS);
        let body = SnapshotManifestBody {
            schema_version: 1,
            snapshot_id,
            repository_id: identity.repository_id().clone(),
            common_dir_sha256: identity.common_dir_sha256().clone(),
            repository_root: identity.root().to_string_lossy().into_owned(),
            common_dir: identity.common_dir().to_string_lossy().into_owned(),
            base_commit,
            selection,
            index_sha256,
            entries,
            index_entries,
            status: status_facts,
            changes,
            omissions,
            warnings,
            warnings_truncated,
        };
        let canonical_bytes = manifest_bytes(&body)?;
        let manifest_sha256 = digest(&canonical_bytes)?;
        Ok(SnapshotManifest {
            body,
            manifest_sha256,
            canonical_bytes,
        })
    }
}
