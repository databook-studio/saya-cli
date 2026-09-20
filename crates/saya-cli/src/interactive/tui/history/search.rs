//! Case-insensitive substring search over history, newest first.

use super::History;

#[allow(dead_code)]
impl History {
    /// Entries matching `needle` (case-insensitive substring), newest first.
    pub(crate) fn search(&self, needle: &str) -> Vec<String> {
        let needle = needle.to_lowercase();
        self.entries
            .iter()
            .rev()
            .filter(|entry| entry.to_lowercase().contains(&needle))
            .cloned()
            .collect()
    }
}
