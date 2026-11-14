//! The search half of the contained workspace surface: `glob` over the
//! minimal in-crate wildcard matcher, and literal-substring `grep` — no
//! regex, so the model cannot confuse a pattern with an execution. Both run
//! over the contained walk (`walk`), so every candidate they see is a path
//! the containment layer itself resolved and re-validated; the search never
//! builds a path it then trusts. A match list past the match bound is served
//! short with the rest counted, never refused: a silently truncated list
//! would read as the whole result, but so would a failed call the model
//! answers by narrowing at random. The visited bound still refuses — a walk
//! cut short cannot even count what it never reached.

use std::str;

use crate::HarnessError;

use super::contain::{EntryKind, MAX_IO_BYTES, MAX_LIST_ENTRIES, Workspace};
use super::pattern::Pattern;

/// One path [`Workspace::glob`] matched, relative to the workspace root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobMatch {
    pub path: String,
}

/// What one [`Workspace::glob`] matched. `omitted` counts the matches the
/// match bound left out — exact, because the walk that found them ran to
/// completion; a walk the visited bound cut short refuses instead of
/// reporting a count it never reached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobOutcome {
    pub matches: Vec<GlobMatch>,
    /// Whether the match bound left matches out.
    pub truncated: bool,
    /// Matches the match bound left out.
    pub omitted: u64,
}

/// One literal-substring hit in one file, as [`Workspace::grep`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrepMatch {
    /// The file, relative to the workspace root.
    pub path: String,
    /// The 1-based line number of the hit.
    pub line: usize,
    /// The line's text, capped at the line bound.
    pub text: String,
    /// Whether `text` was capped at the line bound.
    pub truncated: bool,
}

/// What one [`Workspace::grep`] covered. `files_skipped` counts candidate
/// files the search never read — larger than the per-file read bound, or not
/// valid UTF-8 — so a miss is never confused with proof of absence, and
/// binary content is never served as mojibake. `omitted` counts the hits the
/// match bound left out — exact, because the walk that found them ran to
/// completion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrepOutcome {
    pub matches: Vec<GrepMatch>,
    pub files_scanned: usize,
    pub files_skipped: usize,
    /// Whether the match bound left hits out.
    pub truncated: bool,
    /// Hits the match bound left out.
    pub omitted: u64,
}

impl Workspace {
    /// Matches every real path under the workspace — files and directories
    /// alike, never a symlink — against `pattern`. Past `max_visited` the
    /// walk refuses: it cannot count what it never reached. A match list past
    /// `max_matches` is served short with the rest counted in `omitted` —
    /// the model gets the first `max_matches` paths and the size of what it
    /// did not see, never a failed call.
    pub fn glob(
        &self,
        pattern: &str,
        max_visited: usize,
        max_matches: usize,
    ) -> Result<GlobOutcome, HarnessError> {
        if max_visited > MAX_LIST_ENTRIES {
            return Err(HarnessError::BoundsExceeded {
                path: pattern.to_string(),
                found: max_visited as u64,
                max: MAX_LIST_ENTRIES as u64,
            });
        }
        if max_matches > MAX_LIST_ENTRIES {
            return Err(HarnessError::BoundsExceeded {
                path: pattern.to_string(),
                found: max_matches as u64,
                max: MAX_LIST_ENTRIES as u64,
            });
        }
        let matcher = Pattern::new(pattern);
        let mut matches = Vec::new();
        let mut omitted = 0u64;
        self.walk(max_visited, &mut |rel, _kind, _size| {
            if matcher.matches(rel) {
                if matches.len() < max_matches {
                    matches.push(GlobMatch {
                        path: rel.to_string(),
                    });
                } else {
                    // Past the match bound the walk keeps going so the count
                    // is exact: "500 of 700" is a narrower question, while a
                    // refusal is the same call again with a guess.
                    omitted += 1;
                }
            }
            Ok(())
        })?;
        matches.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(GlobOutcome {
            matches,
            truncated: omitted > 0,
            omitted,
        })
    }

    /// Reports every line of every scannable file under the workspace that
    /// contains `needle` as a literal substring. Each hit names its file,
    /// 1-based line, and the line — itself capped at `max_line_bytes` so one
    /// enormous line cannot blow the context. A file larger than
    /// `max_bytes_per_file` is skipped whole, never half-searched: a hit list
    /// over a prefix would read as full coverage. A file that is not valid
    /// UTF-8 is skipped rather than served as mojibake. Both land in
    /// `files_skipped`, keeping a miss distinguishable from proof of absence.
    /// A hit list past `max_matches` is served short with the rest counted in
    /// `omitted`, never refused. The visited bound still refuses: a walk cut
    /// short cannot count the hits it never reached.
    pub fn grep(
        &self,
        needle: &str,
        case_insensitive: bool,
        max_visited: usize,
        max_matches: usize,
        max_bytes_per_file: u64,
        max_line_bytes: usize,
    ) -> Result<GrepOutcome, HarnessError> {
        if max_visited > MAX_LIST_ENTRIES {
            return Err(HarnessError::BoundsExceeded {
                path: needle.to_string(),
                found: max_visited as u64,
                max: MAX_LIST_ENTRIES as u64,
            });
        }
        if max_matches > MAX_LIST_ENTRIES {
            return Err(HarnessError::BoundsExceeded {
                path: needle.to_string(),
                found: max_matches as u64,
                max: MAX_LIST_ENTRIES as u64,
            });
        }
        if max_bytes_per_file > MAX_IO_BYTES as u64 {
            return Err(HarnessError::BoundsExceeded {
                path: needle.to_string(),
                found: max_bytes_per_file,
                max: MAX_IO_BYTES as u64,
            });
        }
        if max_line_bytes > MAX_IO_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: needle.to_string(),
                found: max_line_bytes as u64,
                max: MAX_IO_BYTES as u64,
            });
        }
        let folded = case_insensitive.then(|| needle.to_lowercase());
        let mut outcome = GrepOutcome::default();
        self.walk(max_visited, &mut |rel, kind, _size| {
            if kind != EntryKind::File {
                return Ok(());
            }
            // A per-file failure here is a race artifact (a swap between the
            // walk and the read), not an escape: the read fails closed inside
            // the containment layer, so the file is skipped and counted —
            // never silently pretended absent, never served from a link.
            let Ok(file) = self.read(rel, max_bytes_per_file) else {
                outcome.files_skipped += 1;
                return Ok(());
            };
            if file.truncated {
                outcome.files_skipped += 1;
                return Ok(());
            }
            let Ok(text) = str::from_utf8(&file.bytes) else {
                outcome.files_skipped += 1;
                return Ok(());
            };
            outcome.files_scanned += 1;
            for (index, line) in text.lines().enumerate() {
                let hit = match &folded {
                    Some(folded_needle) => line.to_lowercase().contains(folded_needle),
                    None => line.contains(needle),
                };
                if hit {
                    if outcome.matches.len() < max_matches {
                        outcome.matches.push(GrepMatch {
                            path: rel.to_string(),
                            line: index + 1,
                            text: bound_line(line, max_line_bytes),
                            truncated: line.len() > max_line_bytes,
                        });
                    } else {
                        // Past the match bound the walk keeps going so the
                        // count is exact: "500 of 700" is a narrower question,
                        // while a refusal is the same call again with a guess.
                        outcome.omitted += 1;
                    }
                }
            }
            Ok(())
        })?;
        outcome.truncated = outcome.omitted > 0;
        outcome
            .matches
            .sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        Ok(outcome)
    }
}

/// Caps one reported line at `max_bytes`, cutting on a character boundary so
/// the bound never splits a multi-byte character into mojibake.
pub(super) fn bound_line(line: &str, max_bytes: usize) -> String {
    if line.len() <= max_bytes {
        return line.to_string();
    }
    let mut cut = max_bytes;
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line[..cut].to_string()
}
