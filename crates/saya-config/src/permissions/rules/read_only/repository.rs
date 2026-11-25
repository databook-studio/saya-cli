//! The repository's own configuration: what a vetted `git` is made to run.
//!
//! `git status` runs a filesystem monitor (`core.fsmonitor`), and `git diff`,
//! `log -p`, and `show` run an external diff (`diff.external`), a textconv
//! driver (`*.textconv`), or a filter's process (`filter.*.process`); a pager
//! and an ssh command are programs of the repository's choosing too. All of
//! them come from the repository's local configuration, which containment
//! treats as ordinary content: the approval layer puts a change inside `.git`
//! to the operator in every mode but `bypass`, but an approved write — or any
//! write under `bypass` — can change it, so a vetted `git` cannot be vetted
//! over it without reading it.
//!
//! The reading is bounded and fails closed: no workspace, no repository
//! inside it, a `.git` that is a pointer file, a configuration that cannot be
//! read or is past the bound, and a configuration that includes another file
//! all keep the question.

use std::path::Path;

use super::WorkingDir;

/// The largest configuration this reads. A real one is a few kilobytes; the
/// bound keeps a repository from making the decision expensive.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Whether the repository the command's `git` segments would read names a
/// program of its own — so the command must ask rather than be vetted.
pub(super) fn runs_programs(dir: WorkingDir<'_>, git_dir: Option<&str>) -> bool {
    let Some(start) = dir.starts_in(git_dir) else {
        // No workspace, or a directory this rule cannot place: the repository
        // `git` would read is unknown.
        return true;
    };
    let Some(root) = dir.root() else {
        return true;
    };
    // `git` searches from where it runs up to the top of the workspace; past
    // that the repository — and its configuration — is outside what this reads.
    let mut current = Some(start.as_path());
    while let Some(directory) = current {
        if !directory.starts_with(root) {
            return true;
        }
        match read(directory) {
            Reading::Runs => return true,
            Reading::RunsNothing => return false,
            Reading::None => {}
        }
        if directory == root {
            break;
        }
        current = directory.parent();
    }
    true
}

/// What one directory holds.
enum Reading {
    /// A repository whose configuration names a program, or one that cannot be
    /// read at all.
    Runs,
    /// A repository whose configuration names none.
    RunsNothing,
    /// No repository here, so `git` would look in the next directory up.
    None,
}

/// Reads `<directory>/.git/config`, bounded.
fn read(directory: &Path) -> Reading {
    // The metadata is taken without following: a `.git` that is a symlink
    // points at a repository this reading is not bounded to, so it is not read.
    let metadata = match std::fs::symlink_metadata(directory.join(".git")) {
        Ok(metadata) => metadata,
        Err(_) => return Reading::None,
    };
    // A `.git` pointer names a configuration this bound cannot reach (a
    // worktree, a submodule), and a symlink names one outside the workspace.
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Reading::Runs;
    }
    match read_bounded(&directory.join(".git/config")) {
        Some(config) if runs_a_program(&config) => Reading::Runs,
        Some(_) => Reading::RunsNothing,
        None => Reading::Runs,
    }
}

/// The configuration's text, or `None` when it cannot be read whole: missing,
/// unreadable, not UTF-8, or past [`MAX_CONFIG_BYTES`].
fn read_bounded(path: &Path) -> Option<String> {
    use std::io::Read;

    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > MAX_CONFIG_BYTES {
        return None;
    }
    Some(text)
}

/// Whether the configuration names a program `git` would run on the
/// repository's behalf, or includes another file that could.
fn runs_a_program(config: &str) -> bool {
    let mut section = String::new();
    for line in config.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            section = header
                .split(']')
                .next()
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            // The rest of the configuration is in a file this reading cannot
            // see.
            if section == "include" || section == "includeif" {
                return true;
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        let key = key.trim().to_ascii_lowercase();
        let runs = match section.as_str() {
            "core" => matches!(key.as_str(), "fsmonitor" | "pager" | "sshcommand"),
            "diff" => key == "external" || key == "textconv",
            "filter" => key == "process",
            _ => false,
        };
        // A `textconv` belongs to a driver section of any name.
        if runs || key.ends_with("textconv") {
            return true;
        }
    }
    false
}
