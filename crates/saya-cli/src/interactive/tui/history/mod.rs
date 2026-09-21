use std::path::PathBuf;

const MAX_ENTRY_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 1000;

/// Persistent, de-duplicated input history with Up/Down navigation.
///
/// Beyond the entries and the navigation `cursor`, this holds the
/// readline-style draft `stash`: the line the user was typing when Up first
/// entered history, restored when Down steps back past the newest entry. The
/// stash is composer view state — never persisted, never part of the session
/// record — and it is cleared by the same two events that end navigation:
/// `push` (a submit) and `reset` (any edit or cursor move after a key).
#[allow(dead_code)]
pub(crate) struct History {
    entries: Vec<String>,
    cursor: Option<usize>,
    stash: Option<String>,
    path: PathBuf,
    limit: usize,
    disabled: bool,
    omitted: usize,
}
#[allow(dead_code)]
impl History {
    pub(crate) fn load() -> Self {
        let path = crate::interactive::session_paths::default_history_file();
        let disabled = persistence::is_disabled_env();
        Self::from_path(path, disabled)
    }

    pub(crate) fn with_path(path: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            stash: None,
            path,
            limit: MAX_ENTRIES,
            disabled: false,
            omitted: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_path_disabled(path: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            stash: None,
            path,
            limit: MAX_ENTRIES,
            disabled: true,
            omitted: 0,
        }
    }

    /// Number of entries omitted by the safety bounds since this history was loaded.
    pub(crate) fn omitted_count(&self) -> usize {
        self.omitted
    }

    /// Test seam: a history pre-seeded with `entries` and persistence off.
    /// Pushes are no-ops (like a disabled history); navigation walks the
    /// seeded entries, so recall is exercisable without touching the disk.
    #[cfg(test)]
    pub(crate) fn with_entries(entries: &[&str]) -> Self {
        Self {
            entries: entries.iter().map(|entry| entry.to_string()).collect(),
            cursor: None,
            stash: None,
            path: PathBuf::new(),
            limit: MAX_ENTRIES,
            disabled: true,
            omitted: 0,
        }
    }
}

fn total_bytes(entries: &[String]) -> usize {
    entries.iter().map(String::len).sum::<usize>() + entries.len().saturating_sub(1)
}

mod persistence;
mod ring;
mod search;

#[cfg(test)]
#[path = "history_search_tests.rs"]
mod search_tests;
#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
