//! The bounded directory listing a workspace summary is built from.
//!
//! It is the workspace's own operation when the run holds one: the same
//! name-sorted, truncation-counting view the `list` tool serves, so a crowded
//! folder names the first entries the model would see. A run without a bound
//! workspace reads the folder itself — up to a fixed cap, then sorted — so the
//! served entries are deterministic either way, never the filesystem's own read
//! order.

use std::fs;
use std::path::Path;
use std::time::Instant;

use saya_harness::workspace::{EntryKind, ListEntry, Workspace};

use super::{EMPTY_FOLDER, Limits, MAX_ENTRIES};

/// The hard cap the plain-folder listing collects before it sorts: four times
/// what one summary names, so a folder without a workspace still serves the
/// first names by name rather than in the filesystem's own read order.
const PLAIN_LIST_CAP: usize = MAX_ENTRIES * 4;

/// One bounded listing: the names served, what it left out, and whether the
/// scan stopped at one of its bounds.
pub(super) struct Listing {
    pub(super) entries: Vec<String>,
    /// Names left out of `entries`; a lower bound unless `exact`.
    pub(super) omitted: u64,
    /// Whether `omitted` is the exact count of names the folder holds.
    pub(super) exact: bool,
    /// The scan stopped at its time bound, or could not read the folder at all.
    pub(super) partial: bool,
}

/// The listing for `folder`, from the workspace when the run holds one.
pub(super) fn of(
    folder: &Path,
    limits: Limits,
    workspace: Option<&Workspace>,
    start: Instant,
) -> Listing {
    match workspace {
        Some(workspace) => list_with_workspace(workspace, limits, start),
        None => list_plain(folder, limits, start),
    }
}

/// The listing the workspace's own operation serves: the same bounded,
/// name-sorted, truncation-counting view the `list` tool gives the model.
fn list_with_workspace(workspace: &Workspace, limits: Limits, start: Instant) -> Listing {
    if start.elapsed() >= limits.time {
        return Listing {
            entries: Vec::new(),
            omitted: 0,
            exact: true,
            partial: true,
        };
    }
    match workspace.list("", limits.entries) {
        Ok(outcome) => Listing {
            entries: outcome.entries.iter().map(marked).collect(),
            omitted: outcome.omitted,
            exact: outcome.omitted_exact,
            partial: outcome.truncated,
        },
        // A folder the workspace cannot list is stated as partial, never as
        // empty: they are different facts about the workspace.
        Err(_) => Listing {
            entries: Vec::new(),
            omitted: 0,
            exact: true,
            partial: true,
        },
    }
}

/// One entry as the summary names it: a directory with its trailing `/`, so
/// the model can tell a file from a folder it may open without a call.
fn marked(entry: &ListEntry) -> String {
    match entry.kind {
        EntryKind::Dir => format!("{}/", entry.name),
        _ => entry.name.clone(),
    }
}

/// The plain-folder listing, for a run with no bound workspace: the read order
/// is the filesystem's, so the names are collected up to a fixed cap, sorted,
/// and only the first `limits.entries` served — deterministic either way.
fn list_plain(folder: &Path, limits: Limits, start: Instant) -> Listing {
    let mut seen: Vec<String> = Vec::new();
    let mut partial = false;
    // Whether the whole folder was read: only then is the omitted count exact.
    let mut complete = true;
    match fs::read_dir(folder) {
        Ok(listing) => {
            for entry in listing.flatten() {
                if start.elapsed() >= limits.time || seen.len() >= PLAIN_LIST_CAP {
                    partial = true;
                    complete = false;
                    break;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
                seen.push(match is_dir {
                    true => format!("{name}/"),
                    false => name,
                });
            }
        }
        // A folder that cannot be listed is stated as partial, never as empty:
        // they are different facts about the workspace.
        Err(_) => partial = true,
    }
    seen.sort();
    let counted = seen.len();
    seen.truncate(limits.entries);
    Listing {
        omitted: (counted - seen.len()) as u64,
        entries: seen,
        exact: complete,
        partial,
    }
}

/// The entry line: the names served, and when the listing stopped at a bound,
/// how many it left out — stated so a short listing never reads as the whole
/// folder.
pub(super) fn entries_line(entries: &[String], omitted: u64, exact: bool, partial: bool) -> String {
    if entries.is_empty() {
        return match partial {
            true => "entries: none read before the bound\n".to_owned(),
            false => format!("entries: none — {EMPTY_FOLDER}\n"),
        };
    }
    let mut line = format!("entries (top level): {}", entries.join(", "));
    if omitted > 0 {
        let suffix = match exact {
            true => format!(" (and {omitted} more)"),
            false => format!(" (and at least {omitted} more)"),
        };
        line.push_str(&suffix);
    }
    line.push('\n');
    line
}
