//! The one rule for a model write that lands in a repository's git directory.
//!
//! A write, edit, delete or notebook edit may target ordinary repository
//! content without a question under `accept-edits`, and the shell may run a
//! vetted command unasked. The git directory is not ordinary content: its
//! configuration and hooks decide what the user's next `git` command runs
//! (`core.hooksPath`, `core.fsmonitor`, `core.sshCommand`, a hook), so a
//! model write there is put to the operator in every mode but `bypass`.
//!
//! Two shapes name that directory, and both are caught here:
//!
//! - a path with a `.git` component — the directory itself, the pointer file a
//!   linked worktree or submodule carries, or a nested repository's;
//! - a path inside the directory a `.git` pointer file names, even when the
//!   target's own spelling has no `.git` component.
//!
//! The reading is lexical and bounded: the pointer file is read up to a small
//! cap, and an unreadable or missing git directory simply adds no second
//! shape. The containment layer still decides whether a write may land at all
//! — this rule only decides whether the operator is asked first.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// The question a write into the git directory is put under: plain, and about
/// the git directory rather than about the tool's own caller.
pub(crate) const QUESTION: &str = "Allow a change inside .git/ (git configuration and hooks)?";

/// The largest `.git` pointer file this reads. A real pointer is one short
/// line; the bound keeps the decision cheap.
const MAX_POINTER_BYTES: u64 = 4096;

/// Whether `rel`, resolved against the checkout at `root`, targets the
/// repository's git directory. `root` is `None` where the composing surface
/// states no checkout; the `.git` component rule still applies then.
pub(crate) fn targets_git_dir(root: Option<&Path>, rel: &str) -> bool {
    let path = Path::new(rel);
    // A `.git` component names the git directory (or the pointer file that
    // stands in for it) in every spelling, whether or not `root` is known.
    if has_git_component(path) {
        return true;
    }
    let Some(root) = root else {
        return false;
    };
    let Some(git_dir) = git_dir(root) else {
        return false;
    };
    let target = if path.is_absolute() {
        lexical_normalize(path)
    } else {
        lexical_normalize(&root.join(path))
    };
    within(&target, &git_dir)
}

/// Whether any component of `path` is `.git`, case-insensitively: git's own
/// name on Windows folds case, and a model may spell it either way.
fn has_git_component(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => name.to_string_lossy().eq_ignore_ascii_case(".git"),
        _ => false,
    })
}

/// The git directory the checkout at `root` names: `.git` itself when it is a
/// directory or a symlink, and the pointer's target when `.git` is a file.
fn git_dir(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    let metadata = fs::symlink_metadata(&dot).ok()?;
    if metadata.file_type().is_symlink() || metadata.is_dir() {
        return Some(fs::canonicalize(&dot).unwrap_or_else(|_| lexical_normalize(&dot)));
    }
    if !metadata.is_file() {
        return None;
    }
    let pointer = read_pointer(&dot)?;
    let named = Path::new(&pointer);
    let target = if named.is_absolute() {
        named.to_path_buf()
    } else {
        root.join(named)
    };
    Some(fs::canonicalize(&target).unwrap_or_else(|_| lexical_normalize(&target)))
}

/// The path a `.git` pointer file names, from its one `gitdir:` line.
fn read_pointer(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_POINTER_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > MAX_POINTER_BYTES {
        return None;
    }
    let line = text.lines().next()?.trim();
    let rest = line.strip_prefix("gitdir:")?.trim();
    (!rest.is_empty()).then(|| rest.to_owned())
}

/// `target` is `dir` or below it. On Windows the comparison folds case, as
/// the filesystem does; elsewhere it is component-exact.
fn within(target: &Path, dir: &Path) -> bool {
    #[cfg(windows)]
    {
        let target = target.to_string_lossy().replace('\\', "/").to_lowercase();
        let dir = dir.to_string_lossy().replace('\\', "/").to_lowercase();
        target == dir
            || target
                .strip_prefix(&dir)
                .is_some_and(|rest| rest.starts_with('/'))
    }
    #[cfg(not(windows))]
    {
        target.starts_with(dir)
    }
}

/// Resolves `.` and `..` without touching the filesystem, so a target that
/// does not exist yet is compared in the same spelling as one that does.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
#[path = "git_write_tests.rs"]
mod tests;
