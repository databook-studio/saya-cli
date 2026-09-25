//! The search half of the contained workspace surface: `glob` over the
//! minimal in-crate wildcard matcher, and literal-substring `grep` — no
//! regex, so the model cannot confuse a pattern with an execution. Both run
//! over the contained walk (`walk`), so every candidate they see is a path
//! the containment layer itself resolved and re-validated; the search never
//! builds a path it then trusts. Bounds fail as typed errors rather than
//! truncating quietly, because a capped "no matches" reads as proof of
//! absence.

use std::str;

use crate::HarnessError;

use super::contain::{EntryKind, MAX_IO_BYTES, MAX_LIST_ENTRIES, Workspace};
use super::pattern::Pattern;

/// One path [`Workspace::glob`] matched, relative to the workspace root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobMatch {
    pub path: String,
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
/// binary content is never served as mojibake.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrepOutcome {
    pub matches: Vec<GrepMatch>,
    pub files_scanned: usize,
    pub files_skipped: usize,
}

impl Workspace {
    /// Matches every real path under the workspace — files and directories
    /// alike, never a symlink — against `pattern`. Both bounds are typed
    /// errors: past `max_visited` the walk refuses, and a match list that
    /// would exceed `max_matches` refuses rather than truncating.
    pub fn glob(
        &self,
        pattern: &str,
        max_visited: usize,
        max_matches: usize,
    ) -> Result<Vec<GlobMatch>, HarnessError> {
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
        self.walk(max_visited, &mut |rel, _kind, _size| {
            if matcher.matches(rel) {
                matches.push(GlobMatch {
                    path: rel.to_string(),
                });
                if matches.len() > max_matches {
                    return Err(HarnessError::BoundsExceeded {
                        path: pattern.to_string(),
                        found: matches.len() as u64,
                        max: max_matches as u64,
                    });
                }
            }
            Ok(())
        })?;
        matches.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(matches)
    }

    /// Reports every line of every scannable file under the workspace that
    /// contains `needle` as a literal substring. Each hit names its file,
    /// 1-based line, and the line — itself capped at `max_line_bytes` so one
    /// enormous line cannot blow the context. A file larger than
    /// `max_bytes_per_file` is skipped whole, never half-searched: a hit list
    /// over a prefix would read as full coverage. A file that is not valid
    /// UTF-8 is skipped rather than served as mojibake. Both land in
    /// `files_skipped`, keeping a miss distinguishable from proof of absence.
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
                    outcome.matches.push(GrepMatch {
                        path: rel.to_string(),
                        line: index + 1,
                        text: bound_line(line, max_line_bytes),
                        truncated: line.len() > max_line_bytes,
                    });
                    if outcome.matches.len() > max_matches {
                        return Err(HarnessError::BoundsExceeded {
                            path: needle.to_string(),
                            found: outcome.matches.len() as u64,
                            max: max_matches as u64,
                        });
                    }
                }
            }
            Ok(())
        })?;
        outcome
            .matches
            .sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        Ok(outcome)
    }
}

/// Caps one reported line at `max_bytes`, cutting on a character boundary so
/// the bound never splits a multi-byte character into mojibake.
fn bound_line(line: &str, max_bytes: usize) -> String {
    if line.len() <= max_bytes {
        return line.to_string();
    }
    let mut cut = max_bytes;
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line[..cut].to_string()
}
