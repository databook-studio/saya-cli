//! The per-file grep scan: which lines (or matches) a file holds, what the
//! query's mode serves for them, and the context lines around each hit.
use super::{state::Scan, types::*};
use std::collections::VecDeque;

impl Scan<'_> {
    /// One readable text file, searched under the query's mode.
    pub(super) fn grep_text(&mut self, path: &str, text: &str) {
        match self.query.grep.mode {
            SearchOutput::Content => {
                if self.multiline_text.is_some() {
                    self.multiline_entries(path, text);
                } else {
                    self.line_entries(path, text);
                }
            }
            SearchOutput::FilesWithMatches => self.file_entry(path, text, true),
            SearchOutput::Count => self.file_entry(path, text, false),
        }
    }
    /// One entry for the whole file: its path for `files_with_matches`, its
    /// path and its own hit count for `count`. `first_only` stops at the first
    /// hit — a names-only answer never needs the rest of the file.
    fn file_entry(&mut self, path: &str, text: &str, first_only: bool) {
        let Some(hits) = self.file_hits(text, first_only) else {
            return;
        };
        if hits == 0 {
            return;
        }
        let hit = if first_only {
            SearchMatch::Path(path.into())
        } else {
            SearchMatch::Count {
                path: path.into(),
                count: hits,
            }
        };
        self.record_found(hit, false);
    }
    /// Hits in one file, or `None` when the walk's own deadline or
    /// cancellation cut the scan short: a partial count is never served as a
    /// whole one, and the outcome is already marked incomplete.
    fn file_hits(&mut self, text: &str, first_only: bool) -> Option<u64> {
        let mut hits = 0;
        if let Some(regex) = self.multiline_text.clone() {
            for _ in regex.find_iter(text.as_bytes()) {
                if !self.alive() {
                    return None;
                }
                hits += 1;
                if first_only {
                    break;
                }
            }
            return Some(hits);
        }
        for line in text.lines() {
            if !self.alive() {
                return None;
            }
            if self.line_matches(line) {
                hits += 1;
                if first_only {
                    break;
                }
            }
        }
        Some(hits)
    }
    /// The line-wise content scan: each hit, with up to `context` unchanged
    /// lines on each side, served once each.
    fn line_entries(&mut self, path: &str, text: &str) {
        let context = self.query.grep.context;
        // The last `context` lines before the current one, held as portions of
        // the file already in hand: no line is copied, and no line the walk
        // has passed is retained.
        let mut before: VecDeque<(usize, &str)> = VecDeque::with_capacity(context);
        let mut after = 0usize;
        // The highest line number already served for this file, so a line
        // between two nearby hits is never served twice.
        let mut served_through = 0usize;
        for (index, line) in text.lines().enumerate() {
            if !self.alive() {
                return;
            }
            let number = index + 1;
            if self.line_matches(line) {
                for (context_number, context_line) in before.drain(..) {
                    if context_number > served_through {
                        self.context_entry(path, context_number, context_line);
                    }
                }
                self.hit_entry(path, number, line);
                served_through = number;
                after = context;
            } else if after > 0 {
                if number > served_through {
                    self.context_entry(path, number, line);
                    served_through = number;
                }
                after -= 1;
            } else if context > 0 {
                if before.len() == context {
                    before.pop_front();
                }
                before.push_back((number, line));
            }
        }
    }
    /// The multiline content scan: the pattern is matched against the whole
    /// file, so a match can span a line boundary. Each match is served at the
    /// line it starts on, with its own span clipped to the line bound.
    fn multiline_entries(&mut self, path: &str, text: &str) {
        let Some(regex) = self.multiline_text.clone() else {
            return;
        };
        let bytes = text.as_bytes();
        // Matches arrive in order and never overlap, so the line of each one
        // is counted from the previous match's start: one pass over the file,
        // whatever the pattern finds.
        let mut cursor = 0usize;
        let mut line = 1usize;
        for found in regex.find_iter(bytes) {
            if !self.alive() {
                return;
            }
            line += bytes[cursor..found.start()]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();
            cursor = found.start();
            let span = &bytes[found.start()..found.end()];
            let clipped = span.len() > self.limits.line_bytes;
            self.record_found(
                SearchMatch::Line {
                    path: path.into(),
                    line,
                    text: bound_span(span, self.limits.line_bytes),
                    truncated: clipped,
                    context: false,
                },
                clipped,
            );
        }
    }
    fn hit_entry(&mut self, path: &str, line: usize, text: &str) {
        let clipped = text.len() > self.limits.line_bytes;
        self.record_found(
            SearchMatch::Line {
                path: path.into(),
                line,
                text: crate::workspace::search::bound_line(text, self.limits.line_bytes),
                truncated: clipped,
                context: false,
            },
            clipped,
        );
    }
    fn context_entry(&mut self, path: &str, line: usize, text: &str) {
        let clipped = text.len() > self.limits.line_bytes;
        self.record_context(
            SearchMatch::Line {
                path: path.into(),
                line,
                text: crate::workspace::search::bound_line(text, self.limits.line_bytes),
                truncated: clipped,
                context: true,
            },
            clipped,
        );
    }
    /// Whether one line matches: the compiled regex, the folded literal, or
    /// the literal itself.
    fn line_matches(&self, line: &str) -> bool {
        if let Some(regex) = &self.text {
            regex.is_match(line.as_bytes())
        } else if let Some(folded) = &self.folded {
            line.to_lowercase().contains(folded)
        } else {
            line.contains(&self.query.pattern)
        }
    }
}

/// Caps one reported match at `max_bytes`, cutting on a character boundary so
/// the bound never splits a multi-byte character into mojibake. The span is
/// bounded before it is decoded: a multiline match can span a whole file.
fn bound_span(span: &[u8], max_bytes: usize) -> String {
    let prefix = &span[..span.len().min(max_bytes.saturating_add(4))];
    crate::workspace::search::bound_line(&String::from_utf8_lossy(prefix), max_bytes)
}
