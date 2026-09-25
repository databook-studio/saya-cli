//! Bounded input-history ring: push with dedup and safety bounds, Up/Down navigation.

use super::{History, MAX_ENTRY_BYTES, MAX_TOTAL_BYTES, total_bytes};

#[allow(dead_code)]
impl History {
    pub(crate) fn push(&mut self, line: &str) -> bool {
        // A submit ends navigation and consumes the draft stash, whatever the
        // entry fate: even a de-duplicated or disabled push must not leave a
        // stash a later Down could resurrect over the freshly cleared line.
        self.stash = None;
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
        // Any edit or cursor move ends navigation: the draft on the line is
        // new, so the stash it was parked from must not come back on a
        // later Down.
        self.stash = None;
    }

    /// True while Up/Down navigation is active: a recalled entry sits on the
    /// input line and `previous`/`next` step from the cursor. False while the
    /// user is typing, or after navigation ended by stepping past an edge.
    pub(crate) fn navigating(&self) -> bool {
        self.cursor.is_some()
    }

    /// Stashes the in-progress draft when navigation begins (the first Up
    /// from an unset cursor). Readline-style: the line the user was typing
    /// survives the walk through history and comes back when Down steps past
    /// the newest entry. View state only — never persisted, never recorded.
    pub(crate) fn stash_draft(&mut self, draft: &str) {
        self.stash = Some(draft.to_string());
    }

    /// Consumes the stashed draft, if any. Taken (not peeked) when Down
    /// restores the live edge, so the stash cannot resurrect after an edit
    /// or a submit has already ended navigation.
    pub(crate) fn take_stash(&mut self) -> Option<String> {
        self.stash.take()
    }
}
