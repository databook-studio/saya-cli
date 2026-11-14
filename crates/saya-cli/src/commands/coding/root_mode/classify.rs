//! The closed classes of managed-only refusals, read from the real error
//! chain. A fallback may absorb only these; cancellation, timeouts, I/O
//! failures, a changed repository, and state-directory errors stay errors.

use super::super::CodingError;
use saya_harness::coding::{
    git::{GitConfigRisk, GitError, GitInventoryError},
    snapshot::SnapshotError,
};

/// The closed class of a managed-only refusal, or `None` when the error is
/// not one a fallback may absorb.
pub(crate) fn managed_only_refusal(error: &CodingError) -> Option<&'static str> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(error);
    // A `Filter` risk read while inventorying the snapshot names an attribute
    // filter (LFS); the same risk read from local config names a configured
    // filter. The chain position is what tells them apart.
    let mut inventorying = false;
    while let Some(error) = current {
        if let Some(snapshot) = error.downcast_ref::<SnapshotError>() {
            match snapshot {
                SnapshotError::UnbornHead => return Some("unborn_head"),
                SnapshotError::ConflictedIndex => return Some("conflicted_index"),
                SnapshotError::UnsupportedFileType => return Some("unsupported_file_type"),
                SnapshotError::FileCountExceeded { .. }
                | SnapshotError::TotalBytesExceeded { .. }
                | SnapshotError::FileBytesExceeded { .. }
                | SnapshotError::ManifestBytesExceeded { .. }
                | SnapshotError::TooLarge => return Some("snapshot_bound"),
                SnapshotError::GitInventory(_) => inventorying = true,
                _ => {}
            }
        }
        if let Some(inventory) = error.downcast_ref::<GitInventoryError>() {
            match inventory {
                GitInventoryError::NestedRepository => return Some("nested_repository"),
                GitInventoryError::UnsupportedWorktree
                | GitInventoryError::UnsupportedWorktreePath => {
                    return Some("unsupported_worktree");
                }
                GitInventoryError::BoundExceeded { .. } => return Some("inventory_bound"),
                GitInventoryError::Git(_) => inventorying = true,
                _ => {}
            }
        }
        if let Some(git) = error.downcast_ref::<GitError>() {
            return match git {
                GitError::BareRepository => Some("bare_repository"),
                GitError::UnsupportedConfiguration(risk) => Some(match risk {
                    GitConfigRisk::Include => "config_include",
                    GitConfigRisk::Filter if inventorying => "lfs_or_filter_attributes",
                    GitConfigRisk::Filter => "config_filter",
                    GitConfigRisk::PartialClone | GitConfigRisk::PromisorRemote => "partial_clone",
                    GitConfigRisk::SparseCheckout => "sparse_checkout",
                    GitConfigRisk::SubmoduleRecursion | GitConfigRisk::Submodule => {
                        "submodule_recursion"
                    }
                }),
                GitError::RepositoryBoundExceeded { bound, .. } if *bound == "attribute files" => {
                    Some("too_many_gitattributes")
                }
                _ => None,
            };
        }
        current = error.source();
    }
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn classifies_managed_only_refusals() {
        let cases: Vec<(CodingError, &'static str)> = vec![
            (
                CodingError::owner(GitError::BareRepository),
                "bare_repository",
            ),
            (
                CodingError::owner(GitError::UnsupportedConfiguration(GitConfigRisk::Include)),
                "config_include",
            ),
            (
                CodingError::owner(GitError::UnsupportedConfiguration(GitConfigRisk::Filter)),
                "config_filter",
            ),
            (
                CodingError::owner(GitError::UnsupportedConfiguration(
                    GitConfigRisk::PartialClone,
                )),
                "partial_clone",
            ),
            (
                CodingError::owner(GitError::UnsupportedConfiguration(
                    GitConfigRisk::SparseCheckout,
                )),
                "sparse_checkout",
            ),
            (
                CodingError::owner(GitError::UnsupportedConfiguration(
                    GitConfigRisk::SubmoduleRecursion,
                )),
                "submodule_recursion",
            ),
            (CodingError::owner(SnapshotError::UnbornHead), "unborn_head"),
            (
                CodingError::owner(SnapshotError::ConflictedIndex),
                "conflicted_index",
            ),
            (
                CodingError::owner(GitInventoryError::NestedRepository),
                "nested_repository",
            ),
            (
                CodingError::owner(GitInventoryError::UnsupportedWorktree),
                "unsupported_worktree",
            ),
            (
                CodingError::owner(SnapshotError::UnsupportedFileType),
                "unsupported_file_type",
            ),
            (
                CodingError::owner(GitInventoryError::BoundExceeded {
                    bound: "directory entries",
                    measured: 10,
                    limit: 1,
                }),
                "inventory_bound",
            ),
            (
                CodingError::owner(SnapshotError::FileCountExceeded {
                    measured: 10,
                    limit: 1,
                }),
                "snapshot_bound",
            ),
            (
                CodingError::owner(SnapshotError::GitInventory(GitInventoryError::Git(
                    GitError::UnsupportedConfiguration(GitConfigRisk::Filter),
                ))),
                "lfs_or_filter_attributes",
            ),
            (
                CodingError::owner(GitError::RepositoryBoundExceeded {
                    bound: "attribute files",
                    measured: 17,
                    limit: 16,
                }),
                "too_many_gitattributes",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(managed_only_refusal(&error), Some(expected), "{error}");
        }
    }

    #[test]
    fn leaves_cancellation_timeouts_io_and_identity_errors_alone() {
        let cases: Vec<CodingError> = vec![
            CodingError::owner(GitError::Cancelled),
            CodingError::owner(GitError::TimedOut),
            CodingError::owner(GitError::RepositoryChanged),
            CodingError::owner(GitError::Spawn(std::io::Error::other("spawn failed"))),
            CodingError::owner(SnapshotError::Cancelled),
            CodingError::owner(SnapshotError::DeadlineExceeded),
            CodingError::StateDirectory,
            CodingError::InvalidRequest,
        ];
        for error in cases {
            assert_eq!(managed_only_refusal(&error), None, "{error}");
        }
    }
}
