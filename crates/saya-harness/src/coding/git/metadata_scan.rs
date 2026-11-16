use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::workspace::Workspace;
use saya_agent::CancellationToken;

use super::parser::MAX_INVENTORY_PATHS;
use super::{
    index_parser::GitIndexEntry, inventory_deadline::InventoryDeadline,
    inventory_types::GitPathStatus, repository::GitInventoryError,
};

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
        // The unix walk has no public-listing ceiling: the inventory's own
        // `remaining` is the bound. The non-unix path goes through
        // `Workspace::list`, whose public ceiling caps one listing; a listing
        // served short is refused below, so a directory the inventory could
        // not count whole never reads as inventoried.
        #[cfg(unix)]
        let requested = remaining;
        #[cfg(not(unix))]
        let requested = remaining.min(crate::workspace::MAX_LIST_ENTRIES);
        #[cfg(unix)]
        let listed = crate::workspace::anchored::list(&workspace, relative, requested);
        #[cfg(not(unix))]
        let listed = workspace.list(relative, requested);
        let listed = match listed {
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
                measured: (entries_seen.saturating_add(requested).saturating_add(1)) as u64,
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
