use std::path::Path;

#[cfg(unix)]
use std::collections::HashSet;
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
use crate::workspace::{Workspace, anchored};
use saya_agent::CancellationToken;

#[cfg(unix)]
use super::parser::MAX_INVENTORY_PATHS;
use super::{
    index_parser::GitIndexEntry, inventory_deadline::InventoryDeadline,
    inventory_types::GitPathStatus, repository::GitInventoryError,
};

#[cfg(unix)]
pub(super) fn reject_nested_repositories(
    root: &Path,
    statuses: &[GitPathStatus],
    indexed_paths: &[GitIndexEntry],
    deadline: InventoryDeadline,
    cancellation: &CancellationToken,
) -> Result<(), GitInventoryError> {
    let mut directories = HashSet::new();
    for status in statuses.iter().filter(|status| !status.ignored) {
        check(deadline, cancellation)?;
        add_ancestors(
            &status.path,
            status.untracked_directory,
            &mut directories,
            deadline,
            cancellation,
        )?;
        if let Some(original) = status.original_path.as_ref() {
            add_ancestors(original, false, &mut directories, deadline, cancellation)?;
        }
    }
    for path in indexed_paths {
        check(deadline, cancellation)?;
        add_ancestors(&path.path, false, &mut directories, deadline, cancellation)?;
    }
    if directories.len() > MAX_INVENTORY_PATHS {
        return Err(GitInventoryError::BoundExceeded {
            bound: "repository directories",
            measured: directories.len() as u64,
            limit: MAX_INVENTORY_PATHS as u64,
        });
    }

    let workspace =
        Workspace::open(root).map_err(|_| GitInventoryError::UnsupportedRepositoryState)?;
    check(deadline, cancellation)?;
    let mut directories: Vec<_> = directories.into_iter().collect();
    directories.sort();
    let mut entries_seen = 0usize;
    for directory in directories {
        check(deadline, cancellation)?;
        let relative = directory
            .to_str()
            .ok_or(GitInventoryError::UnsupportedRepositoryState)?;
        let remaining = MAX_INVENTORY_PATHS.saturating_sub(entries_seen);
        let listed = match anchored::list(&workspace, relative, remaining) {
            Ok(listed) => listed,
            Err(crate::HarnessError::NotFound { .. }) => continue,
            Err(_) => return Err(GitInventoryError::UnsupportedRepositoryState),
        };
        // A directory served short holds more paths than the inventory may
        // carry: the truncation itself is the ceiling, never a subset
        // silently inventoried.
        if listed.truncated {
            return Err(GitInventoryError::BoundExceeded {
                bound: "directory entries",
                measured: (entries_seen.saturating_add(remaining).saturating_add(1)) as u64,
                limit: MAX_INVENTORY_PATHS as u64,
            });
        }
        let measured = entries_seen.saturating_add(listed.entries.len());
        if measured > MAX_INVENTORY_PATHS {
            return Err(GitInventoryError::BoundExceeded {
                bound: "directory entries",
                measured: measured as u64,
                limit: MAX_INVENTORY_PATHS as u64,
            });
        }
        entries_seen = measured;
        if listed
            .entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(".git"))
        {
            return Err(GitInventoryError::NestedRepository);
        }
    }
    check(deadline, cancellation)?;
    Ok(())
}

#[cfg(unix)]
fn add_ancestors(
    path: &Path,
    include_path: bool,
    directories: &mut HashSet<PathBuf>,
    deadline: InventoryDeadline,
    cancellation: &CancellationToken,
) -> Result<(), GitInventoryError> {
    let components: Vec<_> = path.components().collect();
    if components.is_empty() || components.len() > 128 {
        return Err(GitInventoryError::BoundExceeded {
            bound: "path components",
            measured: components.len() as u64,
            limit: 128,
        });
    }
    let mut directory = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        check(deadline, cancellation)?;
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(GitInventoryError::UnsafePath);
        }
        directory.push(component.as_os_str());
        if index + 1 < components.len() || include_path {
            directories.insert(directory.clone());
            if directories.len() > MAX_INVENTORY_PATHS {
                return Err(GitInventoryError::BoundExceeded {
                    bound: "repository directories",
                    measured: directories.len() as u64,
                    limit: MAX_INVENTORY_PATHS as u64,
                });
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn reject_nested_repositories(
    _root: &Path,
    _statuses: &[GitPathStatus],
    _indexed_paths: &[GitIndexEntry],
    _deadline: InventoryDeadline,
    _cancellation: &CancellationToken,
) -> Result<(), GitInventoryError> {
    Err(GitInventoryError::UnsupportedRepositoryState)
}

#[cfg(unix)]
fn check(
    deadline: InventoryDeadline,
    cancellation: &CancellationToken,
) -> Result<(), GitInventoryError> {
    deadline.check()?;
    if cancellation.is_cancelled() {
        Err(GitInventoryError::Git(super::command::GitError::Cancelled))
    } else {
        Ok(())
    }
}
