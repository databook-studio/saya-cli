use std::path::PathBuf;

const MAX_ENTRY_BYTES: usize = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 1000;

/// Persistent, de-duplicated input history with Up/Down navigation.
#[allow(dead_code)]
pub(crate) struct History {
    entries: Vec<String>,
    cursor: Option<usize>,
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
