//! Bounded input-history ring: push with dedup and safety bounds, Up/Down navigation.

use super::{History, MAX_ENTRY_BYTES, MAX_TOTAL_BYTES, total_bytes};

#[allow(dead_code)]
impl History {
    pub(crate) fn push(&mut self, line: &str) -> bool {
        if self.disabled {
            return false;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || self.entries.last().map(String::as_str) == Some(trimmed) {
            return false;
        }
        if trimmed.len() > MAX_ENTRY_BYTES {
            self.omitted += 1;
            return false;
        }
        self.cursor = None;
        self.entries.push(trimmed.to_string());
        if self.entries.len() > self.limit {
            let removed = self.entries.len() - self.limit;
            self.entries.drain(..removed);
            self.omitted += removed;
        }
        while total_bytes(&self.entries) > MAX_TOTAL_BYTES {
            self.entries.remove(0);
            self.omitted += 1;
        }
        self.save();
        true
    }

    pub(crate) fn previous(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = self
            .cursor
            .map_or(self.entries.len() - 1, |i| i.saturating_sub(1));
        self.cursor = Some(idx);
        Some(&self.entries[idx])
    }

    pub(crate) fn next(&mut self) -> Option<&str> {
        let idx = self.cursor?;
        if idx >= self.entries.len().checked_sub(1)? {
            self.cursor = None;
            None
        } else {
            self.cursor = Some(idx + 1);
            Some(&self.entries[idx + 1])
        }
    }

    pub(crate) fn reset(&mut self) {
        self.cursor = None;
    }
}
