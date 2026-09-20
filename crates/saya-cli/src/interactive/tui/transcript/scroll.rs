use super::Transcript;

#[allow(dead_code)]
impl Transcript {
    pub(crate) fn scroll_up(&mut self, n: usize, width: usize, height: usize) {
        let max = self.total_lines(width).saturating_sub(height);
        self.scroll_up = self.scroll_up.saturating_add(n).min(max);
    }

    pub(crate) fn scroll_down(&mut self, n: usize) {
        self.scroll_up = self.scroll_up.saturating_sub(n);
    }

    pub(crate) fn scroll_to_bottom(&mut self) {
        self.scroll_up = 0;
    }

    pub(crate) fn is_following_tail(&self) -> bool {
        self.scroll_up == 0
    }

    pub(crate) fn scroll_metrics(&self, width: usize, height: usize) -> (usize, usize) {
        let total = self.total_lines(width);
        let rem = total.saturating_sub(height);
        if rem == 0 {
            return (total, 0);
        }
        (total, rem - self.scroll_up.min(rem))
    }
}

impl Transcript {
    /// Jumps the viewport to the next line at/after the current top that
    /// contains `needle` (case-insensitive). Returns true when a match was
    /// found. Searching from the tail when following, so repeated searches
    /// walk upward through history.
    pub(crate) fn jump_to_match(&mut self, needle: &str, width: usize, height: usize) -> bool {
        let total = self.total_lines(width);
        if total == 0 || height == 0 {
            return false;
        }
        let needle = needle.to_lowercase();
        let lines = self.lines(width);
        let current_top = total
            .saturating_sub(height)
            .saturating_sub(self.scroll_up.min(total.saturating_sub(height)));
        // Walk downward from just above the current top; wrap once.
        for offset in 0..total {
            let idx = (current_top + offset) % total;
            if lines[idx].is_label {
                continue;
            }
            if lines[idx].text.to_lowercase().contains(&needle) {
                let max_scroll = total.saturating_sub(height);
                self.scroll_up = (total - 1 - idx).min(max_scroll);
                return true;
            }
        }
        false
    }
}

impl Transcript {
    /// Lines containing `needle` (case-insensitive), for the find overlay.
    pub(crate) fn count_matches(&self, needle: &str, width: usize) -> usize {
        if needle.is_empty() {
            return 0;
        }
        let needle = needle.to_lowercase();
        self.lines(width)
            .iter()
            .filter(|row| !row.is_label && row.text.to_lowercase().contains(&needle))
            .count()
    }
}

#[cfg(test)]
#[path = "scroll_tests.rs"]
mod scroll_tests;
