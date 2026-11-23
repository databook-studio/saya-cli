//! The welcome screen's suggestions: the workspace's own top-level names, read
//! once at launch, and the neutral examples a session with nothing to read is
//! offered instead.
//!
//! Every suggestion is built from a name the folder holds — never an invented
//! path — and an empty or unreadable folder falls back to [`NEUTRAL`], which
//! names no path at all.

use std::path::Path;

/// The neutral examples: no name, no path, nothing this session cannot see.
pub(crate) const NEUTRAL: [&str; 3] = [
    "explain how this project is organised",
    "describe what changed recently",
    "walk me through the main entry point",
];

/// At most this many suggestions are offered.
pub(crate) const SHOWN: usize = 3;

/// The most top-level names a launch reads: the welcome needs a few, and a
/// crowded folder must cost one directory listing, not a walk.
const READ: usize = 40;

/// The workspace root's own top-level names: directories with their trailing
/// `/`, names sorted, hidden entries skipped, at most [`READ`] read.
///
/// This is a display fact only: the names reach the welcome screen and nothing
/// else, so a folder that cannot be read simply offers no suggestions.
pub(crate) fn top_level(root: Option<&str>) -> Vec<String> {
    let Some(root) = root else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(Path::new(root)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            Some(if is_dir { format!("{name}/") } else { name })
        })
        .collect();
    names.sort();
    names.truncate(READ);
    names
}

/// The suggestions the welcome screen offers for the top-level `entries`.
///
/// Directories are offered as a question about what they hold; files as a
/// request to explain them. Names that would make a strange suggestion — a
/// path separator, a control character — are skipped rather than rewritten.
pub(crate) fn from_entries(entries: &[String]) -> Vec<String> {
    entries
        .iter()
        .filter(|entry| speaks(entry))
        .take(SHOWN)
        .map(|entry| match entry.strip_suffix('/') {
            Some(directory) => format!("explain what {directory}/ contains"),
            None => format!("explain {entry}"),
        })
        .collect()
}

/// Whether a name can be offered as it is.
fn speaks(entry: &str) -> bool {
    let name = entry.strip_suffix('/').unwrap_or(entry);
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\n', '\r'])
        && name != "/"
}

/// The neutral examples, for a session with no workspace to read.
pub(crate) fn neutral() -> Vec<String> {
    NEUTRAL
        .iter()
        .map(|example| (*example).to_owned())
        .collect()
}

#[cfg(test)]
#[path = "empty_suggestions_tests.rs"]
mod tests;
