use std::path::PathBuf;

/// Persistent, de-duplicated input history with Up/Down navigation.
#[allow(dead_code)]
pub(crate) struct History {
    entries: Vec<String>,
    cursor: Option<usize>,
    path: PathBuf,
    limit: usize,
}

#[allow(dead_code)]
impl History {
    /// Loads history from the default history file path.
    pub(crate) fn load() -> Self {
        let path = crate::interactive::session_paths::default_history_file();
        let entries = match std::fs::read_to_string(&path) {
            Ok(contents) => {
                let mut lines: Vec<String> = contents
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                if lines.len() > 1000 {
                    lines.drain(..lines.len() - 1000);
                }
                lines
            }
            Err(_) => Vec::new(),
        };
        Self {
            entries,
            cursor: None,
            path,
            limit: 1000,
        }
    }

    /// Creates an empty history instance with the given path and limit 1000.
    pub(crate) fn with_path(path: PathBuf) -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
            path,
            limit: 1000,
        }
    }

    /// Pushes a new entry into history, persisting it to disk if changed.
    pub(crate) fn push(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        self.cursor = None;
        if self.entries.last().map(String::as_str) == Some(trimmed) {
            return;
        }
        self.entries.push(trimmed.to_string());
        if self.entries.len() > self.limit {
            self.entries.drain(..self.entries.len() - self.limit);
        }
        self.save();
    }

    /// Moves toward older entries (Up).
    pub(crate) fn previous(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let last_idx = self.entries.len() - 1;
        let new_idx = match self.cursor {
            None => last_idx,
            Some(idx) => idx.saturating_sub(1),
        };
        self.cursor = Some(new_idx);
        Some(&self.entries[new_idx])
    }

    /// Moves toward newer entries (Down).
    pub(crate) fn next(&mut self) -> Option<&str> {
        let idx = self.cursor?;
        let last_idx = self.entries.len().checked_sub(1)?;
        if idx >= last_idx {
            self.cursor = None;
            None
        } else {
            let new_idx = idx + 1;
            self.cursor = Some(new_idx);
            Some(&self.entries[new_idx])
        }
    }

    /// Resets navigation to the live line.
    pub(crate) fn reset(&mut self) {
        self.cursor = None;
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = self.entries.join("\n");
        let _ = std::fs::write(&self.path, content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("saya_history_test_{tag}_{nanos}.txt"))
    }

    #[test]
    fn test_empty_history() {
        let path = temp_file_path("empty");
        let mut history = History::with_path(path.clone());
        assert_eq!(history.previous(), None);
        assert_eq!(history.next(), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_navigation_and_clamping() {
        let path = temp_file_path("nav");
        let mut history = History::with_path(path.clone());
        history.push("first");
        history.push("second");
        history.push("third");

        // Previous walks older then clamps
        assert_eq!(history.previous(), Some("third"));
        assert_eq!(history.previous(), Some("second"));
        assert_eq!(history.previous(), Some("first"));
        assert_eq!(history.previous(), Some("first"));

        // Next walks newer then returns None at live line
        assert_eq!(history.next(), Some("second"));
        assert_eq!(history.next(), Some("third"));
        assert_eq!(history.next(), None);
        assert_eq!(history.next(), None);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_dedup_consecutive() {
        let path = temp_file_path("dedup");
        let mut history = History::with_path(path.clone());
        history.push("cmd");
        history.push("cmd");
        history.push("  cmd  ");
        history.push("other");
        history.push("other");

        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.previous(), Some("other"));
        assert_eq!(history.previous(), Some("cmd"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_persistence_and_reset() {
        let path = temp_file_path("persist");
        {
            let mut history = History::with_path(path.clone());
            history.push("one");
            history.push("two");
            assert_eq!(history.previous(), Some("two"));
            history.reset();
            assert_eq!(history.cursor, None);
        }

        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "one\ntwo");

        let _ = std::fs::remove_file(path);
    }
}
